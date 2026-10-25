// Copyright 2014 Emilie Gillet.
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
// Rust adaptation (OpenPatch Grain Weather) of clouds/dsp/pvoc/stft.h/.cc,
// frame_transformation.h/.cc and phase_vocoder.h/.cc at eurorack
// f4cfa7697095a28d292bff758beb51deb49b5a86, and of the ShyFFT
// (stmlib/fft/shy_fft.h, "largely based on Laurent de Soras' FFTReal") the
// source instantiates as ShyFFT<float, 4096, RotationPhasor>, at stmlib
// e3bd7c9cc00e4364166f9905c0509b6ffd0535ec. The FFT keeps the source's
// butterfly order, workspace use and rotation recurrences, so every binary32
// result matches; no other FFT is substituted.

use super::memory::{Allocator, Memory};
use super::resources::{LUT_SIN, LUT_SINE_WINDOW_4096};
use super::stmlib::{
    clip16, crossfade, fast_atan2r, interpolate, semitones_to_ratio, trunc_i32, Random,
};
use super::Parameters;
use std::cell::Cell;

pub(super) const FFT_SIZE: usize = 4096;
const NUM_PASSES: usize = 12;
const HOP_SIZE: usize = FFT_SIZE / 4;
const STFT_BUFFER_SIZE: usize = FFT_SIZE + HOP_SIZE;
const MAX_NUM_TEXTURES: usize = 7;
const HIGH_FREQUENCY_TRUNCATION: usize = 16;
const TEXTURE_SIZE: usize = (FFT_SIZE >> 1) - HIGH_FREQUENCY_TRUNCATION;
const SQRT_2_DIV_2: f32 = 0.707_106_77;

/// `RotationPhasor<float, 12>::Init`: cosf/sinf of float(pi) / 2^pass for
/// passes 3..11, stored as the binary32 values the source computes.
const ROTATION_LUT: [[u32; 2]; NUM_PASSES - 3] = [
    [0x3f6c_835e, 0x3ec3_ef16],
    [0x3f7b_14be, 0x3e47_c5c2],
    [0x3f7e_c46d, 0x3dc8_bd36],
    [0x3f7f_b10f, 0x3d48_fb30],
    [0x3f7f_ec43, 0x3cc9_0ab0],
    [0x3f7f_fb11, 0x3c49_0e90],
    [0x3f7f_fec4, 0x3bc9_0f88],
    [0x3f7f_ffb1, 0x3b49_0fc6],
    [0x3f7f_ffec, 0x3ac9_0fd6],
];

struct Phasor {
    cos: f32,
    sin: f32,
    real: f32,
    imag: f32,
}

impl Phasor {
    #[inline]
    fn start(pass: usize) -> Self {
        let [c, s] = ROTATION_LUT[pass - 3];
        let (c, s) = (f32::from_bits(c), f32::from_bits(s));
        Self {
            cos: c,
            sin: s,
            real: c,
            imag: s,
        }
    }

    #[inline]
    fn rotate(&mut self) {
        let temp = self.cos * self.real - self.sin * self.imag;
        self.sin = self.cos * self.imag + self.sin * self.real;
        self.cos = temp;
    }
}

#[inline]
fn bit_reverse(i: usize) -> usize {
    // bit_rev_256_lut_ is the 8-bit bit-reversal table.
    let lut = |x: usize| (x as u8).reverse_bits() as usize;
    ((lut(i & 0xff) << 8) | lut(i >> 8)) >> (16 - NUM_PASSES)
}

/// Borrow buffer `source` immutably and the other one mutably.
#[inline]
fn pair<'a>(a: &'a mut [f32], b: &'a mut [f32], source_is_a: bool) -> (&'a [f32], &'a mut [f32]) {
    if source_is_a {
        (a, b)
    } else {
        (b, a)
    }
}

/// `DirectTransform<float, 12, RotationPhasor>`: `input` is the workspace.
fn fft_direct(input: &mut [f32], output: &mut [f32]) {
    let size = FFT_SIZE;
    // First and second pass.
    for i in (0..size).step_by(4) {
        let r0 = bit_reverse(i);
        let r1 = r0 + 2 * (size >> 2);
        let r2 = r0 + (size >> 2);
        let r3 = r0 + 3 * (size >> 2);
        let s = &*input;
        let d = &mut output[i..i + 4];
        d[1] = s[r0] - s[r1];
        d[3] = s[r2] - s[r3];
        let a = s[r0] + s[r1];
        let b = s[r2] + s[r3];
        d[0] = a + b;
        d[2] = a - b;
    }
    // Third pass: s = output, d = input.
    for i in (0..size).step_by(8) {
        let s = &*output;
        let d = &mut *input;
        d[i] = s[i] + s[i + 4];
        d[i + 4] = s[i] - s[i + 4];
        d[i + 2] = s[i + 2];
        d[i + 6] = s[i + 6];
        let v = (s[i + 5] - s[i + 7]) * SQRT_2_DIV_2;
        d[i + 1] = s[i + 1] + v;
        d[i + 3] = s[i + 1] - v;
        let v = (s[i + 5] + s[i + 7]) * SQRT_2_DIV_2;
        d[i + 5] = v + s[i + 3];
        d[i + 7] = v - s[i + 3];
    }
    // Remaining passes; `source_is_input` tracks the pointer flips.
    let mut source_is_input = false;
    for pass in 3..NUM_PASSES {
        source_is_input = !source_is_input;
        let (s, d) = pair(input, output, source_is_input);
        let n = 1 << pass;
        let n_2 = n >> 1;
        for i in (0..size).step_by(n << 1) {
            d[i] = s[i] + s[i + n];
            d[i + n] = s[i] - s[i + n];
            d[i + n_2] = s[i + n_2];
            d[i + n + n_2] = s[i + n + n_2];
            let mut phasor = Phasor::start(pass);
            for j in 1..n_2 {
                let c = phasor.cos;
                let sn = phasor.sin;
                let v = s[i + n + j] * c - s[i + n + n_2 + j] * sn;
                d[i + j] = s[i + j] + v;
                d[i + n - j] = s[i + j] - v;
                let v = s[i + n + j] * sn + s[i + n + n_2 + j] * c;
                d[i + n + j] = v + s[i + n_2 + j];
                d[i + 2 * n - j] = v - s[i + n_2 + j];
                phasor.rotate();
            }
        }
    }
    // Annoying additional data copy step (destination is the input buffer).
    if !source_is_input {
        output.copy_from_slice(input);
    }
}

/// `InverseTransform<float, 12, RotationPhasor>`: `input` is the workspace.
fn fft_inverse(input: &mut [f32], output: &mut [f32]) {
    let size = FFT_SIZE;
    // s = input, d = output.
    let mut destination_is_output = true;
    for pass in (3..NUM_PASSES).rev() {
        let (s, d) = pair(input, output, destination_is_output);
        let n = 1 << pass;
        let n_2 = n >> 1;
        for i in (0..size).step_by(n << 1) {
            d[i] = s[i] + s[i + n];
            d[i + n] = s[i] - s[i + n];
            d[i + n_2] = s[i + n_2] * 2.0;
            d[i + n + n_2] = s[i + n + n_2] * 2.0;
            let mut phasor = Phasor::start(pass);
            for j in 1..n_2 {
                d[i + j] = s[i + j] + s[i + n - j];
                d[i + n_2 + j] = s[i + n + j] - s[i + 2 * n - j];
                let c = phasor.cos;
                let sn = phasor.sin;
                let vr = s[i + j] - s[i + n - j];
                let vi = s[i + n + j] + s[i + 2 * n - j];
                d[i + n + j] = vr * c + vi * sn;
                d[i + n + n_2 + j] = vi * c - vr * sn;
                phasor.rotate();
            }
        }
        destination_is_output = !destination_is_output;
    }
    // Copy data if necessary: when d == output, s is the input buffer.
    if destination_is_output {
        output.copy_from_slice(input);
    }
    // s = output, d = input.
    for i in (0..size).step_by(8) {
        let s = &*output;
        let d = &mut *input;
        d[i] = s[i] + s[i + 4];
        d[i + 4] = s[i] - s[i + 4];
        d[i + 2] = s[i + 2] * 2.0;
        d[i + 6] = s[i + 6] * 2.0;
        d[i + 1] = s[i + 1] + s[i + 3];
        d[i + 3] = s[i + 5] - s[i + 7];
        let vr = s[i + 1] - s[i + 3];
        let vi = s[i + 5] + s[i + 7];
        d[i + 5] = (vr + vi) * SQRT_2_DIV_2;
        d[i + 7] = (vi - vr) * SQRT_2_DIV_2;
    }
    // First and second pass: s = input, d = output.
    for i in (0..size).step_by(4) {
        let r0 = bit_reverse(i);
        let r1 = r0 + 2 * (size >> 2);
        let r2 = r0 + (size >> 2);
        let r3 = r0 + 3 * (size >> 2);
        let s = &input[i..i + 4];
        let b_0 = s[0] + s[2];
        let b_2 = s[0] - s[2];
        let b_1 = s[1] * 2.0;
        let b_3 = s[3] * 2.0;
        output[r0] = b_0 + b_1;
        output[r1] = b_0 - b_1;
        output[r2] = b_2 + b_3;
        output[r3] = b_2 - b_3;
    }
}

const WARP_POLYNOMIALS: [[f32; 4]; 6] = [
    [10.5882, -14.8824, 5.29412, 0.0],
    [-7.3333, 9.0, -1.79167, 0.125],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.5, 0.5, 0.0],
    [-7.3333, 9.5, -2.416667, 0.25],
    [-7.3333, 9.5, -2.416667, 0.25],
];

/// `clouds::FrameTransformation`. Magnitude textures and the phase words
/// live in the memory image; the last texture slot holds the phases and is
/// never cleared, exactly as in the source.
#[derive(Clone, Copy, Debug, Default)]
struct FrameTransformation {
    num_textures: usize,
    textures: [usize; MAX_NUM_TEXTURES],
    phases: usize,
    phases_delta: usize,
    glitch_algorithm: i8,
}

impl FrameTransformation {
    fn init(&mut self, memory: &mut Memory, buffer: usize, num_textures: usize) {
        for i in 0..num_textures {
            self.textures[i] = buffer + 4 * i * TEXTURE_SIZE;
        }
        self.phases = self.textures[num_textures - 1];
        self.num_textures = num_textures - 1;
        self.phases_delta = self.phases + 2 * TEXTURE_SIZE;
        self.glitch_algorithm = 0;
        for i in 0..self.num_textures {
            memory.fill_u32(self.textures[i], TEXTURE_SIZE, 0);
        }
    }

    #[inline]
    fn texture(&self, memory: &Memory, texture: usize, i: usize) -> f32 {
        memory.f32(self.textures[texture] + 4 * i)
    }

    #[inline]
    fn set_texture(&self, memory: &mut Memory, texture: usize, i: usize, value: f32) {
        memory.set_f32(self.textures[texture] + 4 * i, value);
    }

    #[allow(clippy::too_many_arguments)]
    fn process(
        &mut self,
        memory: &mut Memory,
        random: &mut Random,
        parameters: &Parameters,
        fft_out: &mut [f32],
        ifft_in: &mut [f32],
        guard: &Cell<u64>,
    ) {
        fft_out[0] = 0.0;
        fft_out[FFT_SIZE >> 1] = 0.0;
        let freeze = parameters.freeze;
        let glitch = parameters.gate;
        let pitch_ratio = semitones_to_ratio(parameters.pitch);
        if !freeze {
            self.rectangular_to_polar(memory, fft_out);
            self.store_magnitudes(
                memory,
                random,
                fft_out,
                parameters.position,
                parameters.spectral.refresh_rate,
            );
        }
        self.replay_magnitudes(memory, ifft_in, parameters.position);
        Self::warp_magnitudes(ifft_in, fft_out, parameters.spectral.warp, guard);
        Self::shift_magnitudes(fft_out, ifft_in, pitch_ratio, guard);
        if glitch {
            self.add_glitch(random, ifft_in);
        }
        Self::quantize_magnitudes(ifft_in, parameters.spectral.quantization);
        self.set_phases(
            memory,
            random,
            ifft_in,
            parameters.spectral.phase_randomization,
            pitch_ratio,
        );
        Self::polar_to_rectangular(ifft_in);
        if !glitch {
            // Decide on which glitch algorithm will be used next time.
            self.glitch_algorithm = (random.get_sample() & 3) as i8;
        }
        ifft_in[0] = 0.0;
        ifft_in[FFT_SIZE >> 1] = 0.0;
    }

    fn rectangular_to_polar(&self, memory: &mut Memory, fft_data: &mut [f32]) {
        let half = FFT_SIZE >> 1;
        for i in 1..TEXTURE_SIZE {
            let mut magnitude = 0.0;
            let angle = fast_atan2r(fft_data[half + i], fft_data[i], &mut magnitude);
            fft_data[i] = magnitude;
            let previous = memory.u16(self.phases + 2 * i);
            memory.set_u16(self.phases_delta + 2 * i, angle.wrapping_sub(previous));
            memory.set_u16(self.phases + 2 * i, angle);
        }
    }

    fn set_phases(
        &self,
        memory: &mut Memory,
        random: &mut Random,
        destination: &mut [f32],
        phase_randomization: f32,
        pitch_ratio: f32,
    ) {
        let half = FFT_SIZE >> 1;
        for i in 0..TEXTURE_SIZE {
            let phase = memory.u16(self.phases + 2 * i);
            destination[half + i] = f32::from_bits(phase as u32);
            let delta = memory.u16(self.phases_delta + 2 * i);
            // float -> uint16 as compiled for Cortex-M and arm64: saturating
            // conversion to 32 bits, then truncation.
            let advance = (delta as f32 * pitch_ratio) as u32 as u16;
            memory.set_u16(self.phases + 2 * i, phase.wrapping_add(advance));
        }
        let mut r = phase_randomization;
        r = ((r - 0.05) * 1.06).clamp(0.0, 1.0);
        r *= r;
        let amount = trunc_i32(r * 32768.0);
        for i in 0..TEXTURE_SIZE {
            let word = destination[half + i].to_bits();
            let offset = (random.get_sample() as i32).wrapping_mul(amount) >> 14;
            destination[half + i] = f32::from_bits(word.wrapping_add(offset as u32));
        }
    }

    fn polar_to_rectangular(fft_data: &mut [f32]) {
        let half = FFT_SIZE >> 1;
        for i in 1..TEXTURE_SIZE {
            let magnitude = fft_data[i];
            let angle = (fft_data[half + i].to_bits() as u16) >> 6;
            fft_data[i] = magnitude * LUT_SIN[angle as usize + 256];
            fft_data[half + i] = magnitude * LUT_SIN[angle as usize];
        }
        for i in TEXTURE_SIZE..half {
            fft_data[i] = 0.0;
            fft_data[half + i] = 0.0;
        }
    }

    fn add_glitch(&self, random: &mut Random, x: &mut [f32]) {
        let size = TEXTURE_SIZE;
        match self.glitch_algorithm {
            0 => {
                // Spectral hold and blow.
                let mut held = 0.0f32;
                for value in x[..size].iter_mut() {
                    if random.get_sample() & 15 == 0 {
                        held = *value;
                    }
                    *value = held;
                    held *= 1.01;
                }
            }
            1 => {
                // Spectral shift up with aliasing.
                let factor = 1.0 + (random.get_sample() & 7) as f32 / 4.0;
                let mut source = 0.0f32;
                for i in 0..size {
                    source += factor;
                    if source >= size as f32 {
                        source = 0.0;
                    }
                    x[i] = x[trunc_i32(source) as usize];
                }
            }
            2 => {
                // Kill largest harmonic and boost second largest.
                let i = max_element(&x[..size]);
                x[i] = 0.0;
                let i = max_element(&x[..size]);
                x[i] *= 8.0;
            }
            3 => {
                // Nasty high-pass.
                for (i, value) in x[..size].iter_mut().enumerate() {
                    let random = (random.get_sample() as u16 as u32) & 15;
                    if random == 0 {
                        *value *= i as f32 / 16.0;
                    }
                }
            }
            _ => {}
        }
    }

    fn quantize_magnitudes(xf_polar: &mut [f32], amount: f32) {
        let size = TEXTURE_SIZE;
        if amount <= 0.48 {
            let amount = amount * 2.0;
            let scale_down =
                0.5 * semitones_to_ratio(-108.0 * (1.0 - amount * amount)) / FFT_SIZE as f32;
            let scale_up = 1.0 / scale_down;
            for value in xf_polar[..size].iter_mut() {
                *value = scale_up * trunc_i32(scale_down * *value) as f32;
            }
        } else if amount >= 0.52 {
            let amount = (amount - 0.52) * 2.0;
            let norm = xf_polar[max_element(&xf_polar[..size])];
            let inv_norm = 1.0 / (norm + 0.0001);
            for value in xf_polar[1..size].iter_mut() {
                let x = *value * inv_norm;
                let warped = 4.0 * x * (1.0 - x) * (1.0 - x) * (1.0 - x);
                *value = (x + (warped - x) * amount) * norm;
            }
        }
    }

    fn warp_magnitudes(source: &[f32], xf_polar: &mut [f32], amount: f32, guard: &Cell<u64>) {
        let size = TEXTURE_SIZE;
        let bin_width = 1.0 / size as f32;
        let mut f = 0.0f32;
        let amount = amount * 4.0;
        let integral = (trunc_i32(amount) as usize).min(4);
        let fractional = amount - trunc_i32(amount) as f32;
        let mut coefficients = [0.0f32; 4];
        for (i, coefficient) in coefficients.iter_mut().enumerate() {
            *coefficient = crossfade(
                WARP_POLYNOMIALS[integral][i],
                WARP_POLYNOMIALS[integral + 1][i],
                fractional,
            );
        }
        let [a, b, c, d] = coefficients;
        for value in xf_polar[1..size].iter_mut() {
            f += bin_width;
            let wf = (d + f * (c + f * (b + a * f))) * size as f32;
            *value = interpolate(source, wf, 1.0, guard);
        }
    }

    fn shift_magnitudes(source: &[f32], xf_polar: &mut [f32], pitch_ratio: f32, guard: &Cell<u64>) {
        let size = TEXTURE_SIZE;
        let (destination, rest) = xf_polar.split_at_mut(size);
        let temp = &mut rest[..size];
        if pitch_ratio == 1.0 {
            temp.copy_from_slice(&source[..size]);
        } else if pitch_ratio > 1.0 {
            let mut index = 1.0f32;
            let increment = 1.0 / pitch_ratio;
            for value in temp[1..size].iter_mut() {
                *value = interpolate(source, index, 1.0, guard);
                index += increment;
            }
        } else {
            temp.fill(0.0);
            let mut index = 1.0f32;
            let increment = pitch_ratio;
            for &magnitude in &source[1..size] {
                let integral = trunc_i32(index) as usize;
                let fractional = index - integral as f32;
                temp[integral] += (1.0 - fractional) * magnitude;
                temp[integral + 1] += fractional * magnitude;
                index += increment;
            }
        }
        destination.copy_from_slice(temp);
    }

    fn store_magnitudes(
        &self,
        memory: &mut Memory,
        random: &mut Random,
        xf_polar: &[f32],
        position: f32,
        feedback: f32,
    ) {
        let index_float = position * (self.num_textures as f32 - 1.0);
        let index_int = trunc_i32(index_float);
        let index_fractional = index_float - index_int as f32;
        let mut gain_a = 1.0 - index_fractional;
        let mut gain_b = index_fractional;
        let a = index_int as usize;
        let b = a + if position == 1.0 { 0 } else { 1 };
        if feedback >= 0.5 {
            let feedback = 2.0 * (feedback - 0.5);
            if feedback < 0.5 {
                gain_a *= 1.0 - feedback;
                gain_b *= 1.0 - feedback;
                for (i, &x) in xf_polar[..TEXTURE_SIZE].iter().enumerate() {
                    let va = crossfade(self.texture(memory, a, i), x, gain_a);
                    self.set_texture(memory, a, i, va);
                    let vb = crossfade(self.texture(memory, b, i), x, gain_b);
                    self.set_texture(memory, b, i, vb);
                }
            } else {
                let t = (feedback - 0.5) * 0.7 + 0.5;
                let mut gain_new = t - 0.5;
                gain_new = gain_new * gain_new * 2.0 + 0.5;
                let gain_new_a = gain_a * gain_new;
                let gain_new_b = gain_b * gain_new;
                let gain_old_a = 1.0 - gain_a * (1.0 - t);
                let gain_old_b = 1.0 - gain_b * (1.0 - t);
                for (i, &x) in xf_polar[..TEXTURE_SIZE].iter().enumerate() {
                    let va = self.texture(memory, a, i) * gain_old_a + x * gain_new_a;
                    self.set_texture(memory, a, i, va);
                    let vb = self.texture(memory, b, i) * gain_old_b + x * gain_new_b;
                    self.set_texture(memory, b, i, vb);
                }
            }
        } else {
            let mut feedback = feedback * 2.0;
            feedback *= feedback;
            let threshold = (feedback * 65535.0) as u32 as u16;
            for (i, &x) in xf_polar[..TEXTURE_SIZE].iter().enumerate() {
                let gain = if random.get_sample() as u16 <= threshold {
                    1.0
                } else {
                    0.0
                };
                let va = crossfade(self.texture(memory, a, i), x, gain_a * gain);
                self.set_texture(memory, a, i, va);
                let vb = crossfade(self.texture(memory, b, i), x, gain_b * gain);
                self.set_texture(memory, b, i, vb);
            }
        }
    }

    fn replay_magnitudes(&self, memory: &Memory, xf_polar: &mut [f32], position: f32) {
        let index_float = position * (self.num_textures as f32 - 1.0);
        let index_int = trunc_i32(index_float);
        let index_fractional = index_float - index_int as f32;
        let a = index_int as usize;
        let b = a + if position == 1.0 { 0 } else { 1 };
        for (i, value) in xf_polar[..TEXTURE_SIZE].iter_mut().enumerate() {
            *value = crossfade(
                self.texture(memory, a, i),
                self.texture(memory, b, i),
                index_fractional,
            );
        }
    }
}

/// `std::max_element`: index of the first largest element.
fn max_element(values: &[f32]) -> usize {
    let mut largest = 0;
    for (i, value) in values.iter().enumerate().skip(1) {
        if values[largest] < *value {
            largest = i;
        }
    }
    largest
}

/// `clouds::STFT` with its analysis/synthesis ring of shorts in the image.
#[derive(Clone, Copy, Debug, Default)]
struct Stft {
    analysis: usize,
    synthesis: usize,
    buffer_ptr: usize,
    process_ptr: usize,
    block_size: usize,
    ready: usize,
    done: usize,
    has_parameters: bool,
    modifier: FrameTransformation,
}

impl Stft {
    fn init(&mut self, memory: &mut Memory, analysis_synthesis: usize) {
        self.analysis = analysis_synthesis;
        self.synthesis = analysis_synthesis + 2 * STFT_BUFFER_SIZE;
        self.has_parameters = false;
        self.reset(memory);
    }

    fn reset(&mut self, memory: &mut Memory) {
        self.buffer_ptr = 0;
        self.process_ptr = (2 * HOP_SIZE) % STFT_BUFFER_SIZE;
        self.block_size = 0;
        memory.fill_u16(self.analysis, STFT_BUFFER_SIZE, 0);
        memory.fill_u16(self.synthesis, STFT_BUFFER_SIZE, 0);
        self.ready = 0;
        self.done = 0;
    }

    fn process(&mut self, memory: &mut Memory, frames: &mut [[f32; 2]], channel: usize) {
        self.has_parameters = true;
        let mut size = frames.len();
        let mut position = 0;
        while size != 0 {
            let processed = size.min(HOP_SIZE - self.block_size);
            for i in 0..processed {
                let frame = &mut frames[position + i];
                let sample = trunc_i32(frame[channel] * 32768.0);
                memory.set_i16(
                    self.analysis + 2 * (self.buffer_ptr + i),
                    clip16(sample) as i16,
                );
                frame[channel] =
                    memory.i16(self.synthesis + 2 * (self.buffer_ptr + i)) as f32 / 16384.0;
            }
            position += processed;
            self.block_size += processed;
            size -= processed;
            self.buffer_ptr += processed;
            if self.buffer_ptr >= STFT_BUFFER_SIZE {
                self.buffer_ptr -= STFT_BUFFER_SIZE;
            }
            if self.block_size >= HOP_SIZE {
                self.block_size -= HOP_SIZE;
                self.ready += 1;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn buffer(
        &mut self,
        memory: &mut Memory,
        random: &mut Random,
        parameters: &Parameters,
        fft_in: &mut [f32],
        fft_out: &mut [f32],
        guard: &Cell<u64>,
    ) -> bool {
        if self.ready == self.done {
            return false;
        }
        // Copy block to FFT buffer and apply window.
        let mut source_ptr = self.process_ptr;
        for (i, value) in fft_in.iter_mut().enumerate() {
            *value = LUT_SINE_WINDOW_4096[i] * memory.i16(self.analysis + 2 * source_ptr) as f32;
            source_ptr += 1;
            if source_ptr >= STFT_BUFFER_SIZE {
                source_ptr -= STFT_BUFFER_SIZE;
            }
        }
        // Compute FFT. fft_in is lost.
        fft_direct(fft_in, fft_out);
        // Process in the frequency domain: fft_out -> ifft_in (= fft_in).
        if self.has_parameters {
            self.modifier
                .process(memory, random, parameters, fft_out, fft_in, guard);
        } else {
            fft_in.copy_from_slice(fft_out);
        }
        // Compute IFFT into ifft_out (= fft_out). ifft_in is lost.
        fft_inverse(fft_in, fft_out);

        let mut destination_ptr = self.process_ptr;
        let inverse_window_size = 1.0 / ((FFT_SIZE * FFT_SIZE / HOP_SIZE) >> 1) as f32;
        for (i, &value) in fft_out.iter().enumerate() {
            let s = value * LUT_SINE_WINDOW_4096[i] * inverse_window_size;
            let mut x = trunc_i32(s);
            let address = self.synthesis + 2 * destination_ptr;
            if i < FFT_SIZE - HOP_SIZE {
                // Overlap-add.
                x = x.wrapping_add(memory.i16(address) as i32);
            }
            memory.set_i16(address, clip16(x) as i16);
            destination_ptr += 1;
            if destination_ptr >= STFT_BUFFER_SIZE {
                destination_ptr -= STFT_BUFFER_SIZE;
            }
        }
        self.done += 1;
        self.process_ptr += HOP_SIZE;
        if self.process_ptr >= STFT_BUFFER_SIZE {
            self.process_ptr -= STFT_BUFFER_SIZE;
        }
        true
    }
}

/// `clouds::PhaseVocoder`. The source's shared FFT and IFFT buffers are
/// worked on in two preallocated scratch arrays and written back to their
/// addresses in the image after every frame.
#[derive(Clone, Debug)]
pub(super) struct PhaseVocoder {
    num_channels: usize,
    stft: [Stft; 2],
    fft_buffer: usize,
    ifft_buffer: usize,
    scratch_fft: Box<[f32]>,
    scratch_ifft: Box<[f32]>,
}

impl PhaseVocoder {
    pub(super) fn new() -> Self {
        Self {
            num_channels: 0,
            stft: [Stft::default(); 2],
            fft_buffer: 0,
            ifft_buffer: 0,
            scratch_fft: vec![0.0; FFT_SIZE].into_boxed_slice(),
            scratch_ifft: vec![0.0; FFT_SIZE].into_boxed_slice(),
        }
    }

    pub(super) fn reset(&mut self) {
        self.num_channels = 0;
        self.stft = [Stft::default(); 2];
        self.fft_buffer = 0;
        self.ifft_buffer = 0;
        self.scratch_fft.fill(0.0);
        self.scratch_ifft.fill(0.0);
    }

    pub(super) fn copy_from(&mut self, other: &Self) {
        self.num_channels = other.num_channels;
        self.stft = other.stft;
        self.fft_buffer = other.fft_buffer;
        self.ifft_buffer = other.ifft_buffer;
        self.scratch_fft.copy_from_slice(&other.scratch_fft);
        self.scratch_ifft.copy_from_slice(&other.scratch_ifft);
    }

    pub(super) fn init(
        &mut self,
        memory: &mut Memory,
        buffer: [usize; 2],
        buffer_size: [usize; 2],
        num_channels: usize,
    ) {
        self.num_channels = num_channels;
        let mut allocators = [
            Allocator::new(buffer[0], buffer_size[0]),
            Allocator::new(buffer[1], buffer_size[1]),
        ];
        // The configurations Clouds uses always fit.
        self.fft_buffer = allocators[0].allocate(4, FFT_SIZE).unwrap_or(buffer[0]);
        self.ifft_buffer = allocators[num_channels - 1]
            .allocate(4, FFT_SIZE)
            .unwrap_or(buffer[0]);
        let mut num_textures = MAX_NUM_TEXTURES;
        for i in 0..num_channels {
            let analysis_synthesis = allocators[i]
                .allocate(2, (FFT_SIZE + (FFT_SIZE >> 1)) * 2)
                .unwrap_or(buffer[i]);
            num_textures = num_textures.min(allocators[i].free() / (4 * TEXTURE_SIZE));
            self.stft[i].init(memory, analysis_synthesis);
        }
        for i in 0..num_channels {
            let textures = allocators[i]
                .allocate(4, num_textures * TEXTURE_SIZE)
                .unwrap_or(buffer[i]);
            self.stft[i].modifier.init(memory, textures, num_textures);
        }
    }

    pub(super) fn process(&mut self, memory: &mut Memory, frames: &mut [[f32; 2]]) {
        for channel in 0..self.num_channels {
            self.stft[channel].process(memory, frames, channel);
        }
    }

    pub(super) fn buffer(
        &mut self,
        memory: &mut Memory,
        random: &mut Random,
        parameters: &Parameters,
        guard: &Cell<u64>,
    ) {
        for channel in 0..self.num_channels {
            if self.stft[channel].buffer(
                memory,
                random,
                parameters,
                &mut self.scratch_fft,
                &mut self.scratch_ifft,
                guard,
            ) {
                memory.store_f32(self.fft_buffer, &self.scratch_fft);
                memory.store_f32(self.ifft_buffer, &self.scratch_ifft);
            }
        }
    }
}
