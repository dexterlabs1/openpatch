//! Original OpenPatch host-rate bridge for the fixed 96 kHz Warps kernel. MIT.
//!
//! The Warps oscillator pitch tables, filter-bank coefficients and converter
//! FIRs are all designed for 96 kHz, so the kernel always runs there. At other
//! host rates two windowed-sinc resamplers (Blackman-Harris, 32 zero crossings
//! per side, cutoff 0.45 x the lower rate) carry audio in and out. Exact integer
//! time bookkeeping keeps the ratio drift-free; at 48 kHz every phase lands on
//! a stored table row. Tables and rings are sized when the module is built;
//! processing, reset and state adoption allocate nothing.

const ZERO_CROSSINGS: f64 = 32.0;
const PHASES: usize = 256;

#[derive(Debug)]
struct Resampler {
    input_rate: u64,
    output_rate: u64,
    /// Kernel half-width in input samples.
    half_width: f64,
    /// Fixed input delay, in input samples, that keeps the filter causal.
    delay: u64,
    /// h(|t|) sampled every 1/PHASES input samples, one guard entry at the end.
    table: Vec<f32>,
    history: Vec<[f32; 2]>,
    received: u64,
    produced: u64,
}

impl Resampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        let cutoff = 0.45 * input_rate.min(output_rate) as f64 / input_rate as f64;
        let half_width = ZERO_CROSSINGS / (2.0 * cutoff);
        let length = (half_width * PHASES as f64).ceil() as usize + 2;
        let mut table: Vec<f32> = (0..length)
            .map(|k| {
                let t = k as f64 / PHASES as f64;
                if t >= half_width {
                    return 0.0;
                }
                let x = 2.0 * cutoff * t;
                let sinc = if x == 0.0 {
                    1.0
                } else {
                    (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                };
                let w = std::f64::consts::PI * t / half_width;
                let window = 0.35875
                    + 0.48829 * w.cos()
                    + 0.14128 * (2.0 * w).cos()
                    + 0.01168 * (3.0 * w).cos();
                (2.0 * cutoff * sinc * window) as f32
            })
            .collect();
        // Normalize the integer-phase response to exact unity DC gain.
        let mut sum = table[0] as f64;
        let mut k = PHASES;
        while k < length {
            sum += 2.0 * table[k] as f64;
            k += PHASES;
        }
        for value in &mut table {
            *value = (*value as f64 / sum) as f32;
        }
        let ratio = (input_rate as f64 / output_rate as f64).ceil() as usize;
        let history = 2 * half_width.ceil() as usize + ratio + 4;
        Self {
            input_rate: input_rate as u64,
            output_rate: output_rate as u64,
            half_width,
            delay: half_width.ceil() as u64,
            table,
            history: vec![[0.0; 2]; history],
            received: 0,
            produced: 0,
        }
    }

    fn reset(&mut self) {
        self.history.fill([0.0; 2]);
        self.received = 0;
        self.produced = 0;
    }

    fn copy_state_from(&mut self, other: &Self) {
        self.history.copy_from_slice(&other.history);
        self.received = other.received;
        self.produced = other.produced;
    }

    fn push(&mut self, frame: [f32; 2]) {
        let index = (self.received % self.history.len() as u64) as usize;
        self.history[index] = frame;
        self.received += 1;
    }

    #[inline]
    fn tap(&self, t: f64) -> f32 {
        let position = t.abs() * PHASES as f64;
        let index = position as usize;
        if index + 1 >= self.table.len() {
            return 0.0;
        }
        let fraction = (position - index as f64) as f32;
        let a = self.table[index];
        a + (self.table[index + 1] - a) * fraction
    }

    /// Next output frame, if every input it needs has arrived.
    fn pull(&mut self) -> Option<[f32; 2]> {
        // Output m sits at input time m * in / out, delayed by `delay` input
        // samples; every tap then lies at or before floor(m * in / out).
        let numerator = self.produced * self.input_rate;
        if numerator >= self.received * self.output_rate {
            return None;
        }
        let base = (numerator / self.output_rate) as i64 - self.delay as i64;
        let fraction = (numerator % self.output_rate) as f64 / self.output_rate as f64;
        let first = (fraction - self.half_width).ceil() as i64;
        let last = (fraction + self.half_width).floor() as i64;
        let length = self.history.len() as i64;
        let newest = self.received as i64 - 1;
        let mut result = [0.0f32; 2];
        for k in first..=last {
            let j = base + k;
            if j < 0 || j > newest || newest - j >= length {
                continue;
            }
            let weight = self.tap(fraction - k as f64);
            let frame = self.history[(j % length) as usize];
            result[0] += frame[0] * weight;
            result[1] += frame[1] * weight;
        }
        self.produced += 1;
        Some(result)
    }
}

/// Host-rate <-> 96 kHz bridge around a block kernel.
#[derive(Debug)]
pub(super) struct Bridge {
    up: Option<Resampler>,
    down: Option<Resampler>,
    /// Host output FIFO with a fixed pre-roll so bursty block output never
    /// underruns; constant latency follows.
    fifo: Vec<[f32; 2]>,
    read: usize,
    count: usize,
    preroll: usize,
}

impl Bridge {
    pub(super) fn new(host_rate: u32, kernel_rate: u32, block: usize) -> Self {
        let direct = host_rate == kernel_rate;
        let host_per_block = (block as f64 * host_rate as f64 / kernel_rate as f64).ceil() as usize;
        let preroll = host_per_block + 3;
        let mut bridge = Self {
            up: (!direct).then(|| Resampler::new(host_rate, kernel_rate)),
            down: (!direct).then(|| Resampler::new(kernel_rate, host_rate)),
            fifo: vec![[0.0; 2]; 2 * preroll + 8],
            read: 0,
            count: 0,
            preroll,
        };
        bridge.reset();
        bridge
    }

    /// Total host-sample latency from input to output.
    #[cfg(test)]
    pub(super) fn latency(&self) -> f64 {
        let resampling = match (&self.up, &self.down) {
            (Some(up), Some(down)) => {
                up.delay as f64
                    + down.delay as f64 * down.output_rate as f64 / down.input_rate as f64
            }
            _ => 0.0,
        };
        self.preroll as f64 + resampling
    }

    pub(super) fn reset(&mut self) {
        if let Some(up) = &mut self.up {
            up.reset();
        }
        if let Some(down) = &mut self.down {
            down.reset();
        }
        self.fifo.fill([0.0; 2]);
        self.read = 0;
        self.count = self.preroll;
    }

    pub(super) fn copy_state_from(&mut self, other: &Self) {
        if let (Some(a), Some(b)) = (&mut self.up, &other.up) {
            a.copy_state_from(b);
        }
        if let (Some(a), Some(b)) = (&mut self.down, &other.down) {
            a.copy_state_from(b);
        }
        self.fifo.copy_from_slice(&other.fifo);
        self.read = other.read;
        self.count = other.count;
    }

    /// Accept one host input frame; `kernel_input` receives every resulting
    /// kernel-rate frame (zero, one or several).
    pub(super) fn push_input(&mut self, frame: [f32; 2], mut kernel_input: impl FnMut([f32; 2])) {
        match &mut self.up {
            None => kernel_input(frame),
            Some(up) => {
                up.push(frame);
                while let Some(resampled) = up.pull() {
                    kernel_input(resampled);
                }
            }
        }
    }

    /// Accept one kernel-rate output frame.
    pub(super) fn push_output(&mut self, frame: [f32; 2]) {
        let Some(down) = self.down.as_mut() else {
            self.enqueue(frame);
            return;
        };
        down.push(frame);
        while let Some(resampled) = self.down.as_mut().and_then(Resampler::pull) {
            self.enqueue(resampled);
        }
    }

    fn enqueue(&mut self, frame: [f32; 2]) {
        let capacity = self.fifo.len();
        if self.count == capacity {
            // Unreachable with the sized pre-roll; drop the oldest frame.
            self.read = (self.read + 1) % capacity;
            self.count -= 1;
        }
        self.fifo[(self.read + self.count) % capacity] = frame;
        self.count += 1;
    }

    /// One host output frame (zero only if the pipeline ever ran dry).
    pub(super) fn pop(&mut self) -> [f32; 2] {
        if self.count == 0 {
            return [0.0; 2];
        }
        let frame = self.fifo[self.read];
        self.read = (self.read + 1) % self.fifo.len();
        self.count -= 1;
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Host -> 96 kHz -> host with an identity kernel.
    fn round_trip(rate: u32, input: &[f32]) -> (Vec<f32>, f64) {
        let mut bridge = Bridge::new(rate, 96000, 60);
        let mut pending = Vec::with_capacity(16);
        let mut output = Vec::with_capacity(input.len());
        for &x in input {
            pending.clear();
            bridge.push_input([x, -x], |frame| pending.push(frame));
            for frame in &pending {
                bridge.push_output(*frame);
            }
            let frame = bridge.pop();
            assert_eq!(frame[0], -frame[1]);
            output.push(frame[0]);
        }
        (output, bridge.latency())
    }

    #[test]
    fn impulse_latency_is_constant_and_reported() {
        for rate in [8000, 44100, 48000, 88200, 96000, 192000] {
            let mut impulse = vec![0.0; rate as usize / 10];
            impulse[100] = 1.0;
            let (out, latency) = round_trip(rate, &impulse);
            let peak = out
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0;
            assert!(
                (peak as f64 - 100.0 - latency).abs() <= 1.0,
                "rate {rate}: peak at {peak}, latency {latency}"
            );
        }
    }

    #[test]
    fn passband_is_flat_and_dc_is_unity() {
        for rate in [44100u32, 48000] {
            let n = rate as usize;
            for (frequency, tolerance_db) in [(0.0, 0.001), (1000.0, 0.01), (16000.0, 0.1)] {
                let input: Vec<f32> = (0..n)
                    .map(|i| {
                        (std::f64::consts::TAU * frequency * i as f64 / rate as f64).cos() as f32
                            * 0.5
                    })
                    .collect();
                let (out, _) = round_trip(rate, &input);
                let tail = &out[n / 2..];
                let peak = tail.iter().fold(0.0f32, |m, x| m.max(x.abs()));
                let db = 20.0 * (peak as f64 / 0.5).log10();
                assert!(
                    db.abs() <= tolerance_db,
                    "rate {rate} {frequency} Hz: {db} dB"
                );
            }
        }
    }
}
