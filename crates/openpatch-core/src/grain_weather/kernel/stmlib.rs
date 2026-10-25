// Copyright 2012-2014 Emilie Gillet.
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
// Rust adaptation (OpenPatch Grain Weather) of the stmlib helpers used by
// Clouds: dsp/dsp.h, dsp/units.h, dsp/rsqrt.h, dsp/atan.h, dsp/filter.h (Svf),
// dsp/cosine_oscillator.h and utils/random.h at stmlib
// e3bd7c9cc00e4364166f9905c0509b6ffd0535ec. Arithmetic order follows the
// source so binary32 results are identical.

use super::resources::{ATAN_LUT, LUT_PITCH_RATIO_HIGH, LUT_PITCH_RATIO_LOW};
use std::cell::Cell;

/// `static_cast<int32_t>(float)` for in-range values (truncation toward zero).
#[inline]
pub(super) fn trunc_i32(x: f32) -> i32 {
    x as i32
}

/// `stmlib::Interpolate` with a guard on the upper neighbour: when the index
/// lands exactly on the last entry the source reads one element past the
/// table but multiplies it by a zero fraction, so the result is the last entry
/// for any finite neighbour. A nonzero fraction past the end is undefined in
/// the source; it is clamped and counted.
#[inline]
pub(super) fn interpolate(table: &[f32], index: f32, size: f32, guard: &Cell<u64>) -> f32 {
    let index = index * size;
    let integral = trunc_i32(index);
    let fractional = index - integral as f32;
    let i = integral as usize;
    if integral < 0 || i >= table.len() {
        guard.set(guard.get() + 1);
        return 0.0;
    }
    let a = table[i];
    match table.get(i + 1) {
        Some(&b) => a + (b - a) * fractional,
        None => {
            if fractional != 0.0 {
                guard.set(guard.get() + 1);
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

#[inline]
pub(super) fn soft_convert(x: f32) -> i16 {
    clip16(trunc_i32(soft_limit(x * 0.5) * 32768.0)) as i16
}

#[inline]
pub(super) fn semitones_to_ratio(semitones: f32) -> f32 {
    let pitch = semitones + 128.0;
    let integral = trunc_i32(pitch);
    let fractional = pitch - integral as f32;
    LUT_PITCH_RATIO_HIGH[integral as usize]
        * LUT_PITCH_RATIO_LOW[trunc_i32(fractional * 256.0) as usize]
}

#[inline]
pub(super) fn fast_rsqrt_carmack(x: f32) -> f32 {
    let threehalfs = 1.5f32;
    let i = 0x5f37_59df_u32.wrapping_sub(x.to_bits() >> 1);
    let y = f32::from_bits(i);
    let x2 = x * 0.5;
    y * (threehalfs - (x2 * y * y))
}

/// `stmlib::fast_atan2r`: 16-bit angle and magnitude.
#[inline]
pub(super) fn fast_atan2r(y: f32, x: f32, r: &mut f32) -> u16 {
    let squared_magnitude = x * x + y * y;
    if squared_magnitude == 0.0 {
        *r = 0.0;
        return 0;
    }
    let rinv = fast_rsqrt_carmack(squared_magnitude);
    *r = rinv * squared_magnitude;
    const SIGN_MASK: u32 = 0x8000_0000;
    let ux_s = SIGN_MASK & x.to_bits();
    let uy_s = SIGN_MASK & y.to_bits();
    let quadrant = ((!ux_s & uy_s) >> 29) | (ux_s >> 30);
    let x = x.abs();
    let y = y.abs();
    let mut angle: u16 = if y > x {
        16384u16.wrapping_sub(ATAN_LUT[(x * rinv * 512.0 + 0.5) as u32 as usize])
    } else {
        ATAN_LUT[(y * rinv * 512.0 + 0.5) as u32 as usize]
    };
    if ux_s ^ uy_s != 0 {
        angle = angle.wrapping_neg();
    }
    angle.wrapping_add((quadrant << 14) as u16)
}

/// `stmlib::Random`: the 32-bit LCG. The firmware keeps one process-global
/// state; each Grain Weather instance owns its own copy.
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
    pub(super) fn get_sample(&mut self) -> i16 {
        (self.get_word() >> 16) as u16 as i16
    }

    #[inline]
    pub(super) fn get_float(&mut self) -> f32 {
        self.get_word() as f32 / 4_294_967_296.0
    }
}

/// `OnePole::tan<FREQUENCY_FAST>`; constants are the binary32 values the
/// source's double-precision initializers narrow to.
#[inline]
fn tan_fast(f: f32) -> f32 {
    let a = f32::from_bits(0x4121_ba8f);
    let b = f32::from_bits(0x425f_2649);
    let pi = f32::from_bits(0x4049_0fdb);
    let f2 = f * f;
    f * (pi + f2 * (a + b * f2))
}

/// `OnePole::tan<FREQUENCY_DIRTY>`.
#[inline]
fn tan_dirty(f: f32) -> f32 {
    let a = f32::from_bits(0x4139_57d7);
    let pi = f32::from_bits(0x4049_0fdb);
    f * (pi + a * f * f)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FilterMode {
    LowPass,
    HighPass,
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
    pub(super) fn init(&mut self) {
        self.g = tan_dirty(0.01);
        self.r = 1.0 / 100.0;
        self.h = 1.0 / (1.0 + self.r * self.g + self.g * self.g);
        self.state_1 = 0.0;
        self.state_2 = 0.0;
    }

    #[inline]
    pub(super) fn set_f_q_fast(&mut self, f: f32, resonance: f32) {
        self.g = tan_fast(f);
        self.r = 1.0 / resonance;
        self.h = 1.0 / (1.0 + self.r * self.g + self.g * self.g);
    }

    #[inline]
    pub(super) fn set(&mut self, other: &Svf) {
        self.g = other.g;
        self.r = other.r;
        self.h = other.h;
    }

    /// Strided in-place processing of one channel of interleaved frames.
    pub(super) fn process(&mut self, mode: FilterMode, data: &mut [[f32; 2]], channel: usize) {
        let (g, r, h) = (self.g, self.r, self.h);
        let mut state_1 = self.state_1;
        let mut state_2 = self.state_2;
        for frame in data.iter_mut() {
            let hp = (frame[channel] - r * state_1 - g * state_1 - state_2) * h;
            let bp = g * hp + state_1;
            state_1 = g * hp + bp;
            let lp = g * bp + state_2;
            state_2 = g * bp + lp;
            frame[channel] = match mode {
                FilterMode::LowPass => lp,
                FilterMode::HighPass => hp,
            };
        }
        self.state_1 = state_1;
        self.state_2 = state_2;
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
