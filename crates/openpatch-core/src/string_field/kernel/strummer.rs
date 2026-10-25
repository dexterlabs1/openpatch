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
// Rust adaptation (OpenPatch String Field) of rings/dsp/strummer.h and
// rings/dsp/onset_detector.h at eurorack
// f4cfa7697095a28d292bff758beb51deb49b5a86.

use super::stmlib::{slope, NaiveSvf};
use super::{PerformanceState, KERNEL_SAMPLE_RATE, MAX_BLOCK_SIZE};

#[derive(Clone, Copy, Debug)]
struct ZScorer {
    coefficient: f32,
    mean: f32,
    variance: f32,
}

impl ZScorer {
    const ZERO: Self = Self {
        coefficient: 0.0,
        mean: 0.0,
        variance: 0.0,
    };

    fn init(&mut self, cutoff: f32) {
        self.coefficient = cutoff;
        self.mean = 0.0;
        self.variance = 0.0;
    }

    #[inline]
    fn update(&mut self, sample: f32) -> f32 {
        let centered = sample - self.mean;
        self.mean += self.coefficient * centered;
        self.variance += self.coefficient * (centered * centered - self.variance);
        centered
    }

    #[inline]
    fn test(&mut self, sample: f32, threshold: f32, absolute_threshold: f32) -> bool {
        let value = self.update(sample);
        value > self.variance.sqrt() * threshold && value > absolute_threshold
    }
}

#[derive(Clone, Copy, Debug)]
struct Compressor {
    attack: f32,
    decay: f32,
    level: f32,
    skew: f32,
}

impl Compressor {
    const ZERO: Self = Self {
        attack: 0.0,
        decay: 0.0,
        level: 0.0,
        skew: 0.0,
    };

    fn init(&mut self, attack: f32, decay: f32, max_gain: f32) {
        self.attack = attack;
        self.decay = decay;
        self.level = 0.0;
        self.skew = 1.0 / max_gain;
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let mut level = self.level;
        for (x, y) in input.iter().zip(output.iter_mut()) {
            slope(&mut level, x.abs(), self.attack, self.decay);
            *y = *x / (self.skew + level);
        }
        self.level = level;
    }
}

const BAND_SIZE: usize = 32;

#[derive(Clone, Copy, Debug)]
struct OnsetDetector {
    compressor: Compressor,
    low_mid_filter: NaiveSvf,
    mid_high_filter: NaiveSvf,
    attack: [f32; 3],
    decay: [f32; 3],
    energy: [f32; 3],
    envelope: [f32; 3],
    onset_df: f32,
    bands: [[f32; BAND_SIZE]; 3],
    z_df: ZScorer,
    inhibit_threshold: f32,
    inhibit_decay: f32,
    inhibit_time: i32,
    inhibit_counter: i32,
}

impl OnsetDetector {
    const ZERO: Self = Self {
        compressor: Compressor::ZERO,
        low_mid_filter: NaiveSvf::ZERO,
        mid_high_filter: NaiveSvf::ZERO,
        attack: [0.0; 3],
        decay: [0.0; 3],
        energy: [0.0; 3],
        envelope: [0.0; 3],
        onset_df: 0.0,
        bands: [[0.0; BAND_SIZE]; 3],
        z_df: ZScorer::ZERO,
        inhibit_threshold: 0.0,
        inhibit_decay: 0.0,
        inhibit_time: 0,
        inhibit_counter: 0,
    };

    fn init(&mut self, low: f32, low_mid: f32, mid_high: f32, decimated_sr: f32, ioi_time: f32) {
        let ioi_f = 1.0 / (ioi_time * decimated_sr);
        self.compressor.init(ioi_f * 10.0, ioi_f * 0.05, 40.0);
        self.low_mid_filter.init();
        self.mid_high_filter.init();
        self.low_mid_filter.set_f_q_dirty(low_mid, 0.5);
        self.mid_high_filter.set_f_q_dirty(mid_high, 0.5);
        self.attack = [low_mid; 3];
        self.decay = [low * 0.25; 3];
        self.envelope = [0.0; 3];
        self.energy = [0.0; 3];
        self.z_df.init(ioi_f * 0.05);
        self.inhibit_time = (ioi_time * decimated_sr) as i32;
        self.inhibit_decay = 1.0 / (ioi_time * decimated_sr);
        self.inhibit_threshold = 0.0;
        self.inhibit_counter = 0;
        self.onset_df = 0.0;
    }

    fn process(&mut self, samples: &[f32]) -> bool {
        let size = samples.len();
        // Automatic gain control.
        let [band_0, _, _] = &mut self.bands;
        self.compressor.process(samples, &mut band_0[..size]);
        // Quick and dirty filter bank: split the signal in three bands. The
        // source splits band 1 in place; copying it first is equivalent.
        let [band_0, band_1, band_2] = &mut self.bands;
        self.mid_high_filter
            .split(&band_0[..size], &mut band_1[..size], &mut band_2[..size]);
        let mut middle = [0.0f32; BAND_SIZE];
        middle[..size].copy_from_slice(&band_1[..size]);
        self.low_mid_filter
            .split(&middle[..size], &mut band_0[..size], &mut band_1[..size]);
        // Low-pass energy and onset detection function (derivative of
        // energy) in each band.
        let mut onset_df = 0.0f32;
        let mut total_energy = 0.0f32;
        for i in 0..3 {
            let s = &self.bands[i];
            let mut energy = 0.0f32;
            let mut envelope = self.envelope[i];
            let increment = 4 >> i;
            let mut j = 0;
            while j < size {
                slope(&mut envelope, s[j] * s[j], self.attack[i], self.decay[i]);
                energy += envelope;
                j += increment;
            }
            energy = energy.sqrt() * increment as f32;
            self.envelope[i] = envelope;
            let derivative = energy - self.energy[i];
            onset_df += derivative + derivative.abs();
            self.energy[i] = energy;
            total_energy += energy;
        }
        self.onset_df += 0.05 * (onset_df - self.onset_df);
        let outlier_in_df = self.z_df.test(self.onset_df, 1.0, 0.01);
        let exceeds_energy_threshold = total_energy >= self.inhibit_threshold;
        let not_inhibited = self.inhibit_counter == 0;
        let has_onset = outlier_in_df && exceeds_energy_threshold && not_inhibited;
        if has_onset {
            self.inhibit_threshold = total_energy * 1.5;
            self.inhibit_counter = self.inhibit_time;
        } else {
            self.inhibit_threshold -= self.inhibit_decay * self.inhibit_threshold;
            if self.inhibit_counter != 0 {
                self.inhibit_counter -= 1;
            }
        }
        has_onset
    }
}

/// `rings::Strummer`: derives the strum flag when the strum input is
/// normalized (from note changes or input onsets) and inhibits retriggers.
#[derive(Clone, Copy, Debug)]
pub(super) struct Strummer {
    previous_note: f32,
    inhibit_counter: i32,
    inhibit_timer: i32,
    onset_detector: OnsetDetector,
}

impl Strummer {
    pub(super) const ZERO: Self = Self {
        previous_note: 0.0,
        inhibit_counter: 0,
        inhibit_timer: 0,
        onset_detector: OnsetDetector::ZERO,
    };

    pub(super) fn init(&mut self, ioi: f32, sr: f32) {
        self.onset_detector.init(
            8.0 / KERNEL_SAMPLE_RATE,
            160.0 / KERNEL_SAMPLE_RATE,
            1600.0 / KERNEL_SAMPLE_RATE,
            sr,
            ioi,
        );
        self.inhibit_timer = (ioi * sr) as i32;
        self.inhibit_counter = 0;
        self.previous_note = 69.0;
    }

    pub(super) fn process(&mut self, input: &[f32], performance_state: &mut PerformanceState) {
        debug_assert!(input.len() <= MAX_BLOCK_SIZE);
        // The firmware always passes its input block, so the detector runs
        // every block.
        let has_onset = self.onset_detector.process(input);
        let note_changed = (performance_state.note - self.previous_note).abs() > 0.4;
        let mut inhibit_timer = self.inhibit_timer;
        if performance_state.internal_strum {
            let has_external_note_cv = !performance_state.internal_note;
            let has_external_exciter = !performance_state.internal_exciter;
            if has_external_note_cv {
                performance_state.strum = note_changed;
            } else if has_external_exciter {
                performance_state.strum = has_onset;
                // Use longer inhibit time for onset detector.
                inhibit_timer *= 4;
            } else {
                // Nothing is connected.
                performance_state.strum = false;
            }
        }
        if self.inhibit_counter != 0 {
            self.inhibit_counter -= 1;
            performance_state.strum = false;
        } else if performance_state.strum {
            self.inhibit_counter = inhibit_timer;
        }
        self.previous_note = performance_state.note;
    }
}
