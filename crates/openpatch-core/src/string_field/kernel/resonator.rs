// Copyright 2015 Emilie Gillet.
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
// Rust adaptation (OpenPatch String Field) of rings/dsp/resonator.h/.cc at
// eurorack f4cfa7697095a28d292bff758beb51deb49b5a86: the modal resonator, a
// bank of up to 64 band-pass filters whose odd and even modes feed the two
// outputs through a cosine pickup-position weighting.

use super::resources::{LUT_4_DECADES, LUT_STIFFNESS};
use super::stmlib::{interpolate, CosineOscillator, Guard, ParameterInterpolator, Svf};
use super::KERNEL_SAMPLE_RATE;

pub(super) const MAX_MODES: i32 = 64;

#[derive(Clone, Copy, Debug)]
pub(super) struct Resonator {
    frequency: f32,
    structure: f32,
    brightness: f32,
    position: f32,
    previous_position: f32,
    damping: f32,
    resolution: i32,
    f: [Svf; MAX_MODES as usize],
}

impl Resonator {
    pub(super) const ZERO: Self = Self {
        frequency: 0.0,
        structure: 0.0,
        brightness: 0.0,
        position: 0.0,
        previous_position: 0.0,
        damping: 0.0,
        resolution: 0,
        f: [Svf::ZERO; MAX_MODES as usize],
    };

    pub(super) fn init(&mut self) {
        for filter in &mut self.f {
            filter.init();
        }
        self.frequency = 220.0 / KERNEL_SAMPLE_RATE;
        self.structure = 0.25;
        self.brightness = 0.5;
        self.damping = 0.3;
        self.position = 0.999;
        self.previous_position = 0.0;
        self.set_resolution(MAX_MODES);
    }

    pub(super) fn set_frequency(&mut self, frequency: f32) {
        self.frequency = frequency;
    }

    pub(super) fn set_structure(&mut self, structure: f32) {
        self.structure = structure;
    }

    pub(super) fn set_brightness(&mut self, brightness: f32) {
        self.brightness = brightness;
    }

    pub(super) fn set_damping(&mut self, damping: f32) {
        self.damping = damping;
    }

    pub(super) fn set_position(&mut self, position: f32) {
        self.position = position;
    }

    pub(super) fn set_resolution(&mut self, resolution: i32) {
        let resolution = resolution - (resolution & 1); // Must be even!
        self.resolution = resolution.min(MAX_MODES);
    }

    fn compute_filters(&mut self, guard: &Guard) -> usize {
        let mut stiffness = interpolate(&LUT_STIFFNESS, self.structure, 256.0, guard);
        let mut harmonic = self.frequency;
        let mut stretch_factor = 1.0f32;
        let mut q = 500.0 * interpolate(&LUT_4_DECADES, self.damping, 256.0, guard);
        let mut brightness_attenuation = 1.0 - self.structure;
        // Reduces the range of brightness when structure is very low, to
        // prevent clipping.
        brightness_attenuation *= brightness_attenuation;
        brightness_attenuation *= brightness_attenuation;
        brightness_attenuation *= brightness_attenuation;
        let brightness = self.brightness * (1.0 - 0.2 * brightness_attenuation);
        let mut q_loss = brightness * (2.0 - brightness) * 0.85 + 0.15;
        let q_loss_damping_rate = self.structure * (2.0 - self.structure) * 0.1;
        let mut num_modes = 0;
        for i in 0..MAX_MODES.min(self.resolution) as usize {
            let mut partial_frequency = harmonic * stretch_factor;
            if partial_frequency >= 0.49 {
                partial_frequency = 0.49;
            } else {
                num_modes = i + 1;
            }
            self.f[i].set_f_q_fast(partial_frequency, 1.0 + partial_frequency * q);
            stretch_factor += stiffness;
            if stiffness < 0.0 {
                // Make sure that the partials do not fold back into negative
                // frequencies.
                stiffness *= 0.93;
            } else {
                // This helps adding a few extra partials in the highest
                // frequencies.
                stiffness *= 0.98;
            }
            // This prevents the highest partials from decaying too fast.
            q_loss += q_loss_damping_rate * (1.0 - q_loss);
            harmonic += self.frequency;
            q *= q_loss;
        }
        num_modes
    }

    pub(super) fn process(
        &mut self,
        input: &[f32],
        out: &mut [f32],
        aux: &mut [f32],
        guard: &Guard,
    ) {
        let num_modes = self.compute_filters(guard);
        let mut position =
            ParameterInterpolator::new(self.previous_position, self.position, input.len());
        for ((x, o), a) in input.iter().zip(out.iter_mut()).zip(aux.iter_mut()) {
            let mut amplitudes = CosineOscillator::ZERO;
            amplitudes.init_approximate(position.next());
            let input = *x * 0.125;
            let mut odd = 0.0f32;
            let mut even = 0.0f32;
            amplitudes.start();
            // Modes are taken in pairs; with an odd count the last pair
            // reads the next filter, which compute_filters also configured.
            let mut i = 0;
            while i < num_modes {
                odd += amplitudes.next() * self.f[i].process_band_pass(input);
                i += 1;
                even += amplitudes.next() * self.f[i].process_band_pass(input);
                i += 1;
            }
            *o = odd;
            *a = even;
        }
        self.previous_position = position.value;
    }
}
