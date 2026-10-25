// Copyright 2012 Emilie Gillet.
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
// Rust adaptation of warps/dsp/oscillator.{h,cc}, warps/dsp/quadrature_oscillator.h,
// the stmlib Svf/OnePole::tan helpers (stmlib/dsp/filter.h) and stmlib::Random.

use super::resources::{
    LUT_MIDI_TO_F_HIGH, LUT_MIDI_TO_F_LOW, LUT_SIN, WAV_BUZZY_I, WAV_BUZZY_Q, WAV_HARMONICS_I,
    WAV_HARMONICS_Q, WAV_SINE_I, WAV_SINE_Q,
};
use super::{clip16, constrain, interpolate, Interpolator};

const TO_FLOAT: f32 = 1.0 / 4294967296.0;
const TO_UINT32: f32 = 4294967296.0;

const SINE: usize = 0;
const TRIANGLE: usize = 1;
const SAW: usize = 2;
const PULSE: usize = 3;

/// stmlib::Random::GetWord on an instance-owned state.
#[inline]
fn random_word(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

/// Source constants such as `3.333314036e-01f * M_PI_POW_3` multiply a
/// binary32 literal by M_PI repeatedly in binary64, then narrow to binary32.
fn pi_power_constant(coefficient: f32, power: usize) -> f32 {
    let mut value = coefficient as f64;
    for _ in 0..power {
        value *= std::f64::consts::PI;
    }
    value as f32
}

#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    g: f32,
    r: f32,
    h: f32,
    state_1: f32,
    state_2: f32,
}

impl Svf {
    fn init(&mut self) {
        // set_f_q<FREQUENCY_DIRTY>(0.01f, 100.0f)
        let f: f32 = 0.01;
        let a = pi_power_constant(3.736e-01, 3);
        self.set_g_q(f * (std::f32::consts::PI + a * f * f), 100.0);
        self.state_1 = 0.0;
        self.state_2 = 0.0;
    }

    fn set_g_q(&mut self, g: f32, resonance: f32) {
        self.g = g;
        self.r = 1.0 / resonance;
        self.h = 1.0 / (1.0 + self.r * self.g + self.g * self.g);
    }

    /// set_f_q<FREQUENCY_ACCURATE>.
    fn set_f_q_accurate(&mut self, f: f32, resonance: f32) {
        let a = pi_power_constant(3.333314036e-01, 3);
        let b = pi_power_constant(1.333923995e-01, 5);
        let c = pi_power_constant(5.33740603e-02, 7);
        let d = pi_power_constant(2.900525e-03, 9);
        let e = pi_power_constant(9.5168091e-03, 11);
        let f2 = f * f;
        let g = f * (std::f32::consts::PI + f2 * (a + f2 * (b + f2 * (c + f2 * (d + f2 * e)))));
        self.set_g_q(g, resonance);
    }

    fn process_low_pass(&mut self, samples: &mut [f32]) {
        let (g, r, h) = (self.g, self.r, self.h);
        let mut state_1 = self.state_1;
        let mut state_2 = self.state_2;
        for sample in samples {
            let hp = (*sample - r * state_1 - g * state_1 - state_2) * h;
            let bp = g * hp + state_1;
            state_1 = g * hp + bp;
            let lp = g * bp + state_2;
            state_2 = g * bp + lp;
            *sample = lp;
        }
        self.state_1 = state_1;
        self.state_2 = state_2;
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Oscillator {
    high: bool,
    phase: f32,
    phase_increment: f32,
    next_sample: f32,
    lp_state: f32,
    hp_state: f32,
    external_input_level: f32,
    one_hertz: f32,
    filter: Svf,
}

#[inline]
fn this_blep_sample(t: f32) -> f32 {
    0.5 * t * t
}

#[inline]
fn next_blep_sample(t: f32) -> f32 {
    let t = 1.0 - t;
    -0.5 * t * t
}

impl Oscillator {
    pub(super) fn init(&mut self, sample_rate: f32) {
        self.one_hertz = 1.0 / sample_rate;
        self.next_sample = 0.0;
        self.phase = 0.0;
        self.phase_increment = 100.0 * self.one_hertz;
        self.hp_state = 0.0;
        self.lp_state = 0.0;
        self.high = false;
        self.external_input_level = 0.0;
        self.filter.init();
    }

    /// Increment from the source's fixed 96 kHz pitch tables.
    pub(super) fn midi_to_increment(midi_pitch: f32) -> f32 {
        let pitch = (midi_pitch * 256.0) as i32;
        let pitch = (32768 + clip16(pitch.wrapping_sub(20480))) as usize;
        LUT_MIDI_TO_F_HIGH[pitch >> 8] * LUT_MIDI_TO_F_LOW[pitch & 0xff]
    }

    pub(super) fn render(
        &mut self,
        shape: usize,
        note: f32,
        modulation: &[f32],
        out: &mut [f32],
        rng: &mut u32,
    ) -> f32 {
        match shape {
            SINE => self.render_sine(note, modulation, out),
            TRIANGLE => self.render_polyblep::<TRIANGLE>(note, modulation, out),
            SAW => self.render_polyblep::<SAW>(note, modulation, out),
            PULSE => self.render_polyblep::<PULSE>(note, modulation, out),
            _ => self.render_noise(note, modulation, out, rng),
        }
    }

    fn render_sine(&mut self, note: f32, modulation: &[f32], out: &mut [f32]) -> f32 {
        let mut phase = self.phase;
        let mut phase_increment = Interpolator::new(
            self.phase_increment,
            Self::midi_to_increment(note),
            out.len(),
        );
        for (sample, m) in out.iter_mut().zip(modulation) {
            phase += phase_increment.next();
            if phase >= 1.0 {
                phase -= 1.0;
            }
            let mut modulated_phase = (phase * TO_UINT32) as u32;
            modulated_phase = modulated_phase.wrapping_add((*m * 0.5 * TO_UINT32) as i32 as u32);
            let integral = (modulated_phase >> 22) as usize;
            let fractional = (modulated_phase << 10) as f32 * TO_FLOAT;
            let a = LUT_SIN[integral];
            let b = LUT_SIN[integral + 1];
            *sample = a + (b - a) * fractional;
        }
        self.phase = phase;
        self.phase_increment = phase_increment.value();
        1.0
    }

    fn render_polyblep<const SHAPE: usize>(
        &mut self,
        note: f32,
        modulation: &[f32],
        out: &mut [f32],
    ) -> f32 {
        let mut phase = self.phase;
        let mut phase_increment = Interpolator::new(
            self.phase_increment,
            Self::midi_to_increment(note),
            out.len(),
        );
        let mut next_sample = self.next_sample;
        let mut high = self.high;
        let mut lp_state = self.lp_state;
        let mut hp_state = self.hp_state;

        for (sample, m) in out.iter_mut().zip(modulation) {
            let mut this_sample = next_sample;
            next_sample = 0.0;

            let mut modulated_increment = phase_increment.next() * (1.0 + *m);
            if modulated_increment <= 0.0 {
                modulated_increment = 1.0e-7_f64 as f32;
            }
            phase += modulated_increment;

            if SHAPE == TRIANGLE {
                if !high && phase >= 0.5 {
                    let t = (phase - 0.5) / modulated_increment;
                    this_sample += this_blep_sample(t);
                    next_sample += next_blep_sample(t);
                    high = true;
                }
                if phase >= 1.0 {
                    phase -= 1.0;
                    let t = phase / modulated_increment;
                    this_sample -= this_blep_sample(t);
                    next_sample -= next_blep_sample(t);
                    high = false;
                }
                let integrator_coefficient = modulated_increment * 0.0625;
                next_sample += if phase < 0.5 { 0.0 } else { 1.0 };
                this_sample = 128.0 * (this_sample - 0.5);
                lp_state += integrator_coefficient * (this_sample - lp_state);
                *sample = lp_state;
            } else {
                if phase >= 1.0 {
                    phase -= 1.0;
                    let t = phase / modulated_increment;
                    this_sample -= this_blep_sample(t);
                    next_sample -= next_blep_sample(t);
                }
                next_sample += phase;
                if SHAPE == SAW {
                    this_sample = this_sample * 2.0 - 1.0;
                    // Slight roll-off of high frequencies near 48 kHz.
                    lp_state += 0.3 * (this_sample - lp_state);
                    *sample = lp_state;
                } else {
                    lp_state += 0.25 * ((hp_state - this_sample) - lp_state);
                    *sample = 4.0 * lp_state;
                    hp_state = this_sample;
                }
            }
        }

        self.high = high;
        self.phase = phase;
        self.next_sample = next_sample;
        self.lp_state = lp_state;
        self.hp_state = hp_state;

        // The return value is evaluated before the interpolator's destructor
        // stores the new increment, so the pulse gain uses the previous one.
        let gain = if SHAPE == PULSE {
            0.025 / (0.0002 + self.phase_increment)
        } else {
            1.0
        };
        self.phase_increment = phase_increment.value();
        gain
    }

    fn duck(&mut self, external: &[f32], destination: &mut [f32]) {
        let mut level = self.external_input_level;
        for (sample, e) in destination.iter_mut().zip(external) {
            let error = *e * *e - level;
            level += if error > 0.0 { 0.01 } else { 0.0001 } * error;
            let mut internal_gain = 1.0 - 32.0 * level;
            if internal_gain <= 0.0 {
                internal_gain = 0.0;
            }
            *sample = *e + internal_gain * (*sample - *e);
        }
        self.external_input_level = level;
    }

    fn render_noise(
        &mut self,
        note: f32,
        modulation: &[f32],
        out: &mut [f32],
        rng: &mut u32,
    ) -> f32 {
        for sample in out.iter_mut() {
            let noise = random_word(rng) as f32 * TO_FLOAT;
            *sample = 2.0 * noise - 1.0;
        }
        self.duck(modulation, out);
        self.filter
            .set_f_q_accurate(Self::midi_to_increment(note) * 4.0, 1.0);
        self.filter.process_low_pass(out);
        1.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct QuadratureOscillator {
    one_hertz: f32,
    phase: f32,
    frequency: f32,
    shape: f32,
}

impl QuadratureOscillator {
    pub(super) fn init(&mut self, sample_rate: f32) {
        self.one_hertz = 1.0 / sample_rate;
        self.frequency = 0.0;
        self.shape = 0.0;
        self.phase = 0.0;
    }

    pub(super) fn render(
        &mut self,
        shape: f32,
        frequency: f32,
        i_out: &mut [f32],
        q_out: &mut [f32],
        guarded: &mut u32,
    ) {
        const TABLES: [&[f32; 1025]; 6] = [
            &WAV_SINE_I,
            &WAV_SINE_Q,
            &WAV_HARMONICS_I,
            &WAV_HARMONICS_Q,
            &WAV_BUZZY_I,
            &WAV_BUZZY_Q,
        ];
        let size = i_out.len();
        let normalized_frequency = constrain(frequency * self.one_hertz, -0.25, 0.25);
        let mut frequency_parameter = Interpolator::new(self.frequency, normalized_frequency, size);
        let mut shape_parameter = Interpolator::new(self.shape, shape, size);
        let mut phase = self.phase;
        for (i_sample, q_sample) in i_out.iter_mut().zip(q_out.iter_mut()) {
            phase += frequency_parameter.next();
            if phase <= 0.0 {
                phase += 1.0;
            } else if phase >= 1.0 {
                phase -= 1.0;
            }
            let shape = shape_parameter.next() * 1.9999;
            let mut shape_integral = shape as i32;
            let shape_fractional = shape - shape_integral as f32;
            if !(0..=1).contains(&shape_integral) {
                *guarded = guarded.saturating_add(1);
                shape_integral = shape_integral.clamp(0, 1);
            }
            let base = 2 * shape_integral as usize;
            let mut iq = [0.0; 2];
            for (component, value) in iq.iter_mut().enumerate() {
                let a = interpolate(TABLES[base + component], phase, 1024.0, guarded);
                let b = interpolate(TABLES[base + 2 + component], phase, 1024.0, guarded);
                *value = a + (b - a) * shape_fractional;
            }
            *i_sample = iq[0];
            *q_sample = iq[1];
        }
        self.phase = phase;
        self.frequency = frequency_parameter.value();
        self.shape = shape_parameter.value();
    }
}
