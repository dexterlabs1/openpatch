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
// Rust adaptation (OpenPatch String Field) of rings/dsp/fm_voice.h/.cc and
// rings/dsp/follower.h at eurorack f4cfa7697095a28d292bff758beb51deb49b5a86:
// a two-operator FM voice whose envelopes follow the excitation's level and
// spectral centroid.

use super::resources::{LUT_FM_FREQUENCY_QUANTIZER, LUT_SINE};
use super::stmlib::{
    interpolate, semitones_to_ratio, slope, Guard, NaiveSvf, ParameterInterpolator,
};
use super::KERNEL_SAMPLE_RATE;

#[derive(Clone, Copy, Debug)]
struct Follower {
    low_mid_filter: NaiveSvf,
    mid_high_filter: NaiveSvf,
    attack: [f32; 3],
    decay: [f32; 3],
    detector: [f32; 3],
    centroid: f32,
}

impl Follower {
    const ZERO: Self = Self {
        low_mid_filter: NaiveSvf::ZERO,
        mid_high_filter: NaiveSvf::ZERO,
        attack: [0.0; 3],
        decay: [0.0; 3],
        detector: [0.0; 3],
        centroid: 0.0,
    };

    fn init(&mut self, low: f32, low_mid: f32, mid_high: f32) {
        self.low_mid_filter.init();
        self.mid_high_filter.init();
        self.low_mid_filter.set_f_q_dirty(low_mid, 0.5);
        self.mid_high_filter.set_f_q_dirty(mid_high, 0.5);
        self.attack[0] = low_mid;
        self.decay[0] = (low_mid * low).sqrt();
        self.attack[1] = (low_mid * mid_high).sqrt();
        self.decay[1] = low_mid;
        self.attack[2] = (mid_high * 0.5).sqrt();
        self.decay[2] = (mid_high * low_mid).sqrt();
        self.detector = [0.0; 3];
        self.centroid = 0.0;
    }

    /// Returns (envelope, centroid).
    #[inline]
    fn process(&mut self, sample: f32) -> (f32, f32) {
        let mut bands = [0.0f32; 3];
        bands[2] = self.mid_high_filter.process_high_pass(sample);
        bands[1] = self
            .low_mid_filter
            .process_high_pass(self.mid_high_filter.lp());
        bands[0] = self.low_mid_filter.lp();

        let mut weighted = 0.0f32;
        let mut total = 0.0f32;
        let mut frequency = 0.0f32;
        for (i, band) in bands.iter().enumerate() {
            slope(
                &mut self.detector[i],
                band.abs(),
                self.attack[i],
                self.decay[i],
            );
            weighted += self.detector[i] * frequency;
            total += self.detector[i];
            frequency += 0.5;
        }

        let error = weighted / (total + 0.001) - self.centroid;
        let coefficient = if error > 0.0 { 0.05 } else { 0.001 };
        self.centroid += error * coefficient;
        (total, self.centroid)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct FmVoice {
    carrier_frequency: f32,
    ratio: f32,
    brightness: f32,
    damping: f32,
    position: f32,
    feedback_amount: f32,
    previous_carrier_frequency: f32,
    previous_modulator_frequency: f32,
    previous_brightness: f32,
    previous_damping: f32,
    previous_feedback_amount: f32,
    amplitude_envelope: f32,
    brightness_envelope: f32,
    gain: f32,
    fm_amount: f32,
    carrier_phase: u32,
    modulator_phase: u32,
    previous_sample: f32,
    follower: Follower,
}

#[inline]
fn sine_fm(phase: u32, fm: f32) -> f32 {
    // `static_cast<uint32_t>` saturates on AArch64, Cortex-M and in Rust.
    let phase = phase.wrapping_add((((fm + 4.0) * 536_870_912.0) as u32) << 3);
    let integral = (phase >> 20) as usize;
    let fractional = (phase << 12) as f32 / 4_294_967_296.0;
    let a = LUT_SINE[integral];
    let b = LUT_SINE[integral + 1];
    a + (b - a) * fractional
}

impl FmVoice {
    pub(super) const ZERO: Self = Self {
        carrier_frequency: 0.0,
        ratio: 0.0,
        brightness: 0.0,
        damping: 0.0,
        position: 0.0,
        feedback_amount: 0.0,
        previous_carrier_frequency: 0.0,
        previous_modulator_frequency: 0.0,
        previous_brightness: 0.0,
        previous_damping: 0.0,
        previous_feedback_amount: 0.0,
        amplitude_envelope: 0.0,
        brightness_envelope: 0.0,
        gain: 0.0,
        fm_amount: 0.0,
        carrier_phase: 0,
        modulator_phase: 0,
        previous_sample: 0.0,
        follower: Follower::ZERO,
    };

    /// `FMVoice::Init`; `previous_sample_` is left as it was, as in the
    /// source.
    pub(super) fn init(&mut self) {
        self.carrier_frequency = 220.0 / KERNEL_SAMPLE_RATE;
        self.ratio = 0.5;
        self.brightness = 0.5;
        self.damping = 0.5;
        self.position = 0.5;
        self.feedback_amount = 0.0;
        self.previous_carrier_frequency = self.carrier_frequency;
        self.previous_modulator_frequency = self.carrier_frequency;
        self.previous_brightness = self.brightness;
        self.previous_damping = self.damping;
        self.previous_feedback_amount = self.feedback_amount;
        self.amplitude_envelope = 0.0;
        self.brightness_envelope = 0.0;
        self.carrier_phase = 0;
        self.modulator_phase = 0;
        self.gain = 0.0;
        self.fm_amount = 0.0;
        self.follower.init(
            8.0 / KERNEL_SAMPLE_RATE,
            160.0 / KERNEL_SAMPLE_RATE,
            1600.0 / KERNEL_SAMPLE_RATE,
        );
    }

    pub(super) fn set_frequency(&mut self, frequency: f32) {
        self.carrier_frequency = frequency;
    }

    pub(super) fn set_ratio(&mut self, ratio: f32) {
        self.ratio = ratio;
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

    pub(super) fn set_feedback_amount(&mut self, feedback_amount: f32) {
        self.feedback_amount = feedback_amount;
    }

    pub(super) fn trigger_internal_envelope(&mut self) {
        self.amplitude_envelope = 1.0;
        self.brightness_envelope = 1.0;
    }

    pub(super) fn process(
        &mut self,
        input: &[f32],
        out: &mut [f32],
        aux: &mut [f32],
        guard: &Guard,
    ) {
        let size = input.len();
        let envelope_amount = if self.damping < 0.9 {
            1.0
        } else {
            (1.0 - self.damping) * 10.0
        };
        let amplitude_rt60 =
            0.1 * semitones_to_ratio(self.damping * 96.0, guard) * KERNEL_SAMPLE_RATE;
        let amplitude_decay = 1.0 - 0.001f32.powf(1.0 / amplitude_rt60);
        let brightness_rt60 =
            0.1 * semitones_to_ratio(self.damping * 84.0, guard) * KERNEL_SAMPLE_RATE;
        let brightness_decay = 1.0 - 0.001f32.powf(1.0 / brightness_rt60);

        let ratio = interpolate(&LUT_FM_FREQUENCY_QUANTIZER, self.ratio, 128.0, guard);
        let mut modulator_frequency = self.carrier_frequency * semitones_to_ratio(ratio, guard);
        if modulator_frequency > 0.5 {
            modulator_frequency = 0.5;
        }

        let feedback = (self.feedback_amount - 0.5) * 2.0;

        let mut carrier_increment = ParameterInterpolator::new(
            self.previous_carrier_frequency,
            self.carrier_frequency,
            size,
        );
        let mut modulator_increment = ParameterInterpolator::new(
            self.previous_modulator_frequency,
            modulator_frequency,
            size,
        );
        let mut brightness =
            ParameterInterpolator::new(self.previous_brightness, self.brightness, size);
        let mut feedback_amount =
            ParameterInterpolator::new(self.previous_feedback_amount, feedback, size);
        let mut carrier_phase = self.carrier_phase;
        let mut modulator_phase = self.modulator_phase;
        let mut previous_sample = self.previous_sample;

        for ((x, o), a) in input.iter().zip(out.iter_mut()).zip(aux.iter_mut()) {
            let (amplitude_envelope, mut brightness_envelope) = self.follower.process(*x);
            brightness_envelope *= 2.0 * amplitude_envelope * (2.0 - amplitude_envelope);

            slope(
                &mut self.amplitude_envelope,
                amplitude_envelope,
                0.05,
                amplitude_decay,
            );
            slope(
                &mut self.brightness_envelope,
                brightness_envelope,
                0.01,
                brightness_decay,
            );

            let mut brightness_value = brightness.next();
            brightness_value *= brightness_value;
            let fm_amount_min = if brightness_value < 0.5 {
                0.0
            } else {
                brightness_value * 2.0 - 1.0
            };
            let fm_amount_max = if brightness_value < 0.5 {
                2.0 * brightness_value
            } else {
                1.0
            };
            let fm_envelope = 0.5 + envelope_amount * (self.brightness_envelope - 0.5);
            let fm_amount = (fm_amount_min + fm_amount_max * fm_envelope) * 2.0;
            // SLEW(fm_amount_, fm_amount, 0.005f + fm_amount_max * 0.015f)
            let delta = 0.005 + fm_amount_max * 0.015;
            let mut error = fm_amount - self.fm_amount;
            if error > delta {
                error = delta;
            } else if error < -delta {
                error = -delta;
            }
            self.fm_amount += error;
            // The block-level feedback: the per-sample value is declared
            // after this line in the source.
            let phase_feedback = if feedback < 0.0 {
                0.5 * feedback * feedback
            } else {
                0.0
            };
            modulator_phase = modulator_phase.wrapping_add(
                (4_294_967_296.0
                    * modulator_increment.next()
                    * (1.0 + previous_sample * phase_feedback)) as u32,
            );
            carrier_phase =
                carrier_phase.wrapping_add((4_294_967_296.0 * carrier_increment.next()) as u32);
            let feedback = feedback_amount.next();
            let modulator_fb = if feedback > 0.0 {
                0.25 * feedback * feedback
            } else {
                0.0
            };
            let modulator = sine_fm(modulator_phase, modulator_fb * previous_sample);
            let carrier = sine_fm(carrier_phase, self.fm_amount * modulator);
            previous_sample += 0.1 * (carrier - previous_sample);
            let gain = 1.0 + envelope_amount * (self.amplitude_envelope - 1.0);
            self.gain += (0.005 + 0.045 * self.fm_amount) * (gain - self.gain);

            *o = (carrier + 0.5 * modulator) * self.gain;
            *a = 0.5 * modulator * self.gain;
        }
        self.carrier_phase = carrier_phase;
        self.modulator_phase = modulator_phase;
        self.previous_sample = previous_sample;
        self.previous_carrier_frequency = carrier_increment.value;
        self.previous_modulator_frequency = modulator_increment.value;
        self.previous_brightness = brightness.value;
        self.previous_feedback_amount = feedback_amount.value;
    }
}
