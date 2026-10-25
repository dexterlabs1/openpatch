// Copyright 2012-2015 Emilie Gillet.
//
// Author: Emilie Gillet (emilie.o.gillet@gmail.com)
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
//
// See http://creativecommons.org/licenses/MIT/ for more information.
//
// Rust adaptation (OpenPatch String Field) of the stmlib helpers used by
// Rings: dsp/dsp.h, dsp/units.h, dsp/filter.h (Svf, NaiveSvf, DCBlocker),
// dsp/delay_line.h, dsp/cosine_oscillator.h, dsp/parameter_interpolator.h and
// utils/random.h at stmlib e3bd7c9cc00e4364166f9905c0509b6ffd0535ec.
// Arithmetic order follows the source so binary32 results are identical.

use super::resources::{LUT_PITCH_RATIO_HIGH, LUT_PITCH_RATIO_LOW};
use std::cell::Cell;

/// Count of reads the source would make outside an object (undefined
/// behaviour in C++); the Rust kernel clamps them instead.
pub(super) type Guard = Cell<u64>;

#[inline]
fn count(guard: &Guard) {
    guard.set(guard.get() + 1);
}

/// `static_cast<int32_t>(float)`: truncation toward zero. Out-of-range
/// values are undefined in C++; AArch64, Cortex-M and Rust all saturate.
#[inline]
pub(super) fn trunc_i32(x: f32) -> i32 {
    x as i32
}

/// `stmlib::Interpolate`. When the index lands exactly on the last entry the
/// source reads one element past the table but multiplies it by a zero
/// fraction, so the result is the last entry for any finite neighbour. A
/// nonzero fraction past the end is undefined in the source; it is clamped
/// and counted.
#[inline]
pub(super) fn interpolate(table: &[f32], index: f32, size: f32, guard: &Guard) -> f32 {
    let index = index * size;
    let integral = trunc_i32(index);
    let fractional = index - integral as f32;
    if integral < 0 || integral as usize >= table.len() {
        count(guard);
        return table[(integral.max(0) as usize).min(table.len() - 1)];
    }
    let i = integral as usize;
    let a = table[i];
    match table.get(i + 1) {
        Some(&b) => a + (b - a) * fractional,
        None => {
            if fractional != 0.0 {
                count(guard);
            }
            a
        }
    }
}

#[inline]
pub(super) fn crossfade(a: f32, b: f32, fade: f32) -> f32 {
    a + (b - a) * fade
}

#[inline]
pub(super) fn soft_limit(x: f32) -> f32 {
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Portable `Clip16` (the source's `TEST` branch).
#[inline]
pub(super) fn clip16(x: i32) -> i32 {
    x.clamp(-32768, 32767)
}

/// `std::min(a, b)`: `(b < a) ? b : a`.
#[inline]
pub(super) fn min(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}

/// `std::max(a, b)`: `(a < b) ? b : a`.
#[inline]
pub(super) fn max(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}

/// `SLOPE(out, in, positive, negative)`.
#[inline]
pub(super) fn slope(out: &mut f32, input: f32, positive: f32, negative: f32) {
    let error = input - *out;
    *out += (if error > 0.0 { positive } else { negative }) * error;
}

/// `stmlib::SemitonesToRatio`. Its tables cover -128..+128 semitones; the
/// source reads outside them beyond that range, which is clamped and counted.
#[inline]
pub(super) fn semitones_to_ratio(semitones: f32, guard: &Guard) -> f32 {
    let pitch = semitones + 128.0;
    let integral = trunc_i32(pitch);
    let fractional = pitch - integral as f32;
    let low = trunc_i32(fractional * 256.0);
    if !(0..256).contains(&integral) || !(0..256).contains(&low) {
        count(guard);
        return LUT_PITCH_RATIO_HIGH[integral.clamp(0, 255) as usize]
            * LUT_PITCH_RATIO_LOW[low.clamp(0, 255) as usize];
    }
    LUT_PITCH_RATIO_HIGH[integral as usize] * LUT_PITCH_RATIO_LOW[low as usize]
}

/// `stmlib::Random`: the 32-bit LCG. The firmware keeps one process-global
/// state; each String Field instance owns its own copy.
#[derive(Clone, Copy, Debug)]
pub(super) struct Random {
    pub(super) state: u32,
}

impl Random {
    #[inline]
    pub(super) fn get_word(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        self.state
    }

    #[inline]
    pub(super) fn get_float(&mut self) -> f32 {
        self.get_word() as f32 / 4_294_967_296.0
    }
}

/// `float(M_PI)`.
pub(super) const PI_F: f32 = f32::from_bits(0x4049_0fdb);

/// `OnePole::tan<FREQUENCY_DIRTY>`; constants are the binary32 values the
/// source's double-precision initializers narrow to.
#[inline]
fn tan_dirty(f: f32) -> f32 {
    let a = f32::from_bits(0x4139_57d7);
    f * (PI_F + a * f * f)
}

/// `OnePole::tan<FREQUENCY_FAST>`.
#[inline]
fn tan_fast(f: f32) -> f32 {
    let a = f32::from_bits(0x4121_ba8f);
    let b = f32::from_bits(0x425f_2649);
    let f2 = f * f;
    f * (PI_F + f2 * (a + b * f2))
}

/// `OnePole::tan<FREQUENCY_ACCURATE>`.
#[inline]
fn tan_accurate(f: f32) -> f32 {
    let a = f32::from_bits(0x4125_5da9);
    let b = f32::from_bits(0x4223_4865);
    let c = f32::from_bits(0x4321_348f);
    let d = f32::from_bits(0x42ac_ec90);
    let e = f32::from_bits(0x452e_fe23);
    let f2 = f * f;
    f * (PI_F + f2 * (a + f2 * (b + f2 * (c + f2 * (d + f2 * e)))))
}

/// `stmlib::Svf`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Svf {
    g: f32,
    r: f32,
    h: f32,
    state_1: f32,
    state_2: f32,
}

impl Svf {
    pub(super) const ZERO: Self = Self {
        g: 0.0,
        r: 0.0,
        h: 0.0,
        state_1: 0.0,
        state_2: 0.0,
    };

    pub(super) fn init(&mut self) {
        self.set_f_q_dirty(0.01, 100.0);
        self.state_1 = 0.0;
        self.state_2 = 0.0;
    }

    #[inline]
    fn set_g_q(&mut self, g: f32, resonance: f32) {
        self.g = g;
        self.r = 1.0 / resonance;
        self.h = 1.0 / (1.0 + self.r * self.g + self.g * self.g);
    }

    #[inline]
    pub(super) fn set_f_q_dirty(&mut self, f: f32, resonance: f32) {
        self.set_g_q(tan_dirty(f), resonance);
    }

    #[inline]
    pub(super) fn set_f_q_fast(&mut self, f: f32, resonance: f32) {
        self.set_g_q(tan_fast(f), resonance);
    }

    #[inline]
    pub(super) fn set_f_q_accurate(&mut self, f: f32, resonance: f32) {
        self.set_g_q(tan_accurate(f), resonance);
    }

    /// One sample; returns (band pass, low pass).
    #[inline]
    fn tick(&mut self, input: f32) -> (f32, f32) {
        let hp = (input - self.r * self.state_1 - self.g * self.state_1 - self.state_2) * self.h;
        let bp = self.g * hp + self.state_1;
        self.state_1 = self.g * hp + bp;
        let lp = self.g * bp + self.state_2;
        self.state_2 = self.g * bp + lp;
        (bp, lp)
    }

    #[inline]
    pub(super) fn process_band_pass(&mut self, input: f32) -> f32 {
        self.tick(input).0
    }

    #[inline]
    pub(super) fn process_low_pass(&mut self, input: f32) -> f32 {
        self.tick(input).1
    }

    /// In-place low-pass block processing.
    pub(super) fn process_low_pass_block(&mut self, data: &mut [f32]) {
        for sample in data.iter_mut() {
            *sample = self.tick(*sample).1;
        }
    }
}

/// `stmlib::NaiveSvf` in its approximate (`FREQUENCY_DIRTY`) mode.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct NaiveSvf {
    f: f32,
    damp: f32,
    lp: f32,
    bp: f32,
}

impl NaiveSvf {
    pub(super) const ZERO: Self = Self {
        f: 0.0,
        damp: 0.0,
        lp: 0.0,
        bp: 0.0,
    };

    pub(super) fn init(&mut self) {
        self.set_f_q_dirty(0.01, 100.0);
        self.lp = 0.0;
        self.bp = 0.0;
    }

    pub(super) fn set_f_q_dirty(&mut self, f: f32, resonance: f32) {
        let f = if f < 0.158 { f } else { 0.158 };
        self.f = 2.0 * PI_F * f;
        self.damp = 1.0 / resonance;
    }

    #[inline]
    pub(super) fn process_high_pass(&mut self, input: f32) -> f32 {
        let bp_normalized = self.bp * self.damp;
        let notch = input - bp_normalized;
        self.lp += self.f * self.bp;
        let hp = notch - self.lp;
        self.bp += self.f * hp;
        hp
    }

    #[inline]
    pub(super) fn lp(&self) -> f32 {
        self.lp
    }

    /// `Split`: low and high outputs of one input block.
    pub(super) fn split(&mut self, input: &[f32], low: &mut [f32], high: &mut [f32]) {
        let mut lp = self.lp;
        let mut bp = self.bp;
        for ((x, l), h) in input.iter().zip(low.iter_mut()).zip(high.iter_mut()) {
            let bp_normalized = bp * self.damp;
            let notch = *x - bp_normalized;
            lp += self.f * bp;
            let hp = notch - lp;
            bp += self.f * hp;
            *l = lp;
            *h = hp;
        }
        self.lp = lp;
        self.bp = bp;
    }
}

/// `stmlib::DCBlocker`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DcBlocker {
    pole: f32,
    x: f32,
    y: f32,
}

impl DcBlocker {
    pub(super) const ZERO: Self = Self {
        pole: 0.0,
        x: 0.0,
        y: 0.0,
    };

    pub(super) fn init(&mut self, pole: f32) {
        self.x = 0.0;
        self.y = 0.0;
        self.pole = pole;
    }

    #[inline]
    pub(super) fn process(&mut self, data: &mut [f32]) {
        let mut x = self.x;
        let mut y = self.y;
        let pole = self.pole;
        for sample in data.iter_mut() {
            let old_x = x;
            x = *sample;
            y = y * pole + x - old_x;
            *sample = y;
        }
        self.x = x;
        self.y = y;
    }
}

/// `stmlib::CosineOscillator` in its approximate mode.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CosineOscillator {
    y1: f32,
    y0: f32,
    iir_coefficient: f32,
    initial_amplitude: f32,
}

impl CosineOscillator {
    pub(super) const ZERO: Self = Self {
        y1: 0.0,
        y0: 0.0,
        iir_coefficient: 0.0,
        initial_amplitude: 0.0,
    };

    /// `Init<COSINE_OSCILLATOR_APPROXIMATE>`, which ends with `Start`.
    pub(super) fn init_approximate(&mut self, frequency: f32) {
        let mut sign = 16.0f32;
        let mut frequency = frequency - 0.25;
        if frequency < 0.0 {
            frequency = -frequency;
        } else if frequency > 0.5 {
            frequency -= 0.5;
        } else {
            sign = -16.0;
        }
        self.iir_coefficient = sign * frequency * (1.0 - 2.0 * frequency);
        self.initial_amplitude = self.iir_coefficient * 0.25;
        self.start();
    }

    #[inline]
    pub(super) fn start(&mut self) {
        self.y1 = self.initial_amplitude;
        self.y0 = 0.5;
    }

    #[inline]
    pub(super) fn value(&self) -> f32 {
        self.y1 + 0.5
    }

    #[inline]
    pub(super) fn next(&mut self) -> f32 {
        let temp = self.y0;
        self.y0 = self.iir_coefficient * self.y0 - self.y1;
        self.y1 = temp;
        temp + 0.5
    }
}

/// `stmlib::ParameterInterpolator`. The source's destructor stores the
/// current value (not the target) back into the state; callers do the same
/// with `value` when the scope ends.
#[derive(Clone, Copy, Debug)]
pub(super) struct ParameterInterpolator {
    pub(super) value: f32,
    increment: f32,
}

impl ParameterInterpolator {
    #[inline]
    pub(super) fn new(state: f32, new_value: f32, size: usize) -> Self {
        Self {
            value: state,
            increment: (new_value - state) / size as f32,
        }
    }

    #[inline]
    pub(super) fn next(&mut self) -> f32 {
        self.value += self.increment;
        self.value
    }
}

/// Read/write positions of a `stmlib::DelayLine<float, N>` whose samples live
/// in separately owned storage. `N` is a power of two, so the source's
/// unsigned wrap-around of negative offsets reduces to the same index.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DelayCursor<const N: usize> {
    write_ptr: usize,
    delay: usize,
}

impl<const N: usize> DelayCursor<N> {
    pub(super) const ZERO: Self = Self {
        write_ptr: 0,
        delay: 0,
    };

    /// `Init`/`Reset`.
    pub(super) fn reset(&mut self, line: &mut [f32; N]) {
        line.fill(0.0);
        self.delay = 1;
        self.write_ptr = 0;
    }

    pub(super) fn set_delay(&mut self, delay: usize) {
        self.delay = delay;
    }

    #[inline]
    fn index(&self, offset: i32) -> usize {
        self.write_ptr.wrapping_add(offset as isize as usize) % N
    }

    #[inline]
    pub(super) fn write(&mut self, line: &mut [f32; N], sample: f32) {
        line[self.write_ptr] = sample;
        self.write_ptr = (self.write_ptr + N - 1) % N;
    }

    #[inline]
    pub(super) fn allpass(
        &mut self,
        line: &mut [f32; N],
        sample: f32,
        delay: usize,
        coefficient: f32,
    ) -> f32 {
        let read = line[self.write_ptr.wrapping_add(delay) % N];
        let write = sample + coefficient * read;
        self.write(line, write);
        -write * coefficient + read
    }

    /// `Read()` at the configured delay.
    #[inline]
    pub(super) fn read_delay(&self, line: &[f32; N]) -> f32 {
        line[self.write_ptr.wrapping_add(self.delay) % N]
    }

    /// `Read(float)`: linear interpolation.
    #[inline]
    pub(super) fn read(&self, line: &[f32; N], delay: f32) -> f32 {
        let integral = trunc_i32(delay);
        let fractional = delay - integral as f32;
        let a = line[self.index(integral)];
        let b = line[self.index(integral.wrapping_add(1))];
        a + (b - a) * fractional
    }

    /// `ReadHermite(float)`.
    #[inline]
    pub(super) fn read_hermite(&self, line: &[f32; N], delay: f32) -> f32 {
        let integral = trunc_i32(delay);
        let fractional = delay - integral as f32;
        // `int32_t t = write_ptr_ + delay_integral + max_delay;`
        let t = self
            .write_ptr
            .wrapping_add(integral as isize as usize)
            .wrapping_add(N) as i32;
        let at = |offset: i32| line[(t.wrapping_add(offset) as isize as usize) % N];
        let xm1 = at(-1);
        let x0 = at(0);
        let x1 = at(1);
        let x2 = at(2);
        let c = (x1 - xm1) * 0.5;
        let v = x0 - x1;
        let w = c + v;
        let a = w + v + (x2 - x0) * 0.5;
        let b_neg = w + a;
        let f = fractional;
        (((a * f) - b_neg) * f + c) * f + x0
    }
}

/// `stmlib::DelayLine<float, N>` with inline storage.
#[derive(Clone, Copy, Debug)]
pub(super) struct DelayLine<const N: usize> {
    pub(super) cursor: DelayCursor<N>,
    pub(super) line: [f32; N],
}

impl<const N: usize> DelayLine<N> {
    pub(super) const ZERO: Self = Self {
        cursor: DelayCursor::ZERO,
        line: [0.0; N],
    };

    pub(super) fn init(&mut self) {
        self.cursor.reset(&mut self.line);
    }

    #[inline]
    pub(super) fn write(&mut self, sample: f32) {
        self.cursor.write(&mut self.line, sample);
    }

    #[inline]
    pub(super) fn read(&self, delay: f32) -> f32 {
        self.cursor.read(&self.line, delay)
    }
}
