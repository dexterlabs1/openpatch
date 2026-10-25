// Copyright 2014 Emilie Gillet.
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
// Rust adaptation of warps/dsp/vocoder.{h,cc}, warps/dsp/filter_bank.{h,cc},
// warps/dsp/limiter.h and stmlib::CrossoverSvf (stmlib/dsp/filter.h).

use super::resources::{FILTER_BANK, SRC_DOWN_3_36, SRC_DOWN_4_48, SRC_UP_3_36, SRC_UP_4_48};
use super::sample_rate::{SrcDown, SrcUp};
use super::{constrain, semitones_to_ratio, soft_limit};

const NUM_BANDS: usize = 20;
const LOW_FACTOR: usize = 4;
const MID_FACTOR: usize = 3;
const DELAY_LINE_SIZE: usize = 6144;
const MAX_FILTER_BANK_BLOCK_SIZE: usize = 96;
const SAMPLE_MEMORY_SIZE: usize = MAX_FILTER_BANK_BLOCK_SIZE * NUM_BANDS / 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    LowPass,
    BandPassNormalized,
    HighPass,
}

/// stmlib::CrossoverSvf: two cascaded SVF stages sharing f/fq.
#[derive(Clone, Copy, Debug, Default)]
struct CrossoverSvf {
    f: f32,
    fq: f32,
    lp: [f32; 2],
    bp: [f32; 2],
    x: [f32; 2],
}

impl CrossoverSvf {
    fn init(&mut self) {
        self.lp = [0.0; 2];
        self.bp = [0.0; 2];
        self.x = [0.0; 2];
    }

    fn process(&mut self, mode: Mode, samples: &mut [f32]) {
        let (mut lp_1, mut bp_1, mut lp_2, mut bp_2) =
            (self.lp[0], self.bp[0], self.lp[1], self.bp[1]);
        let (mut x_1, mut x_2) = (self.x[0], self.x[1]);
        let fq = self.fq;
        let f = self.f;
        for sample in samples {
            let input = *sample;
            lp_1 += f * bp_1;
            bp_1 += -fq * bp_1 - f * lp_1 + input;
            if mode == Mode::BandPassNormalized {
                bp_1 += x_1;
            }
            x_1 = input;
            let y = match mode {
                Mode::LowPass => lp_1 * f,
                Mode::BandPassNormalized => bp_1 * fq,
                Mode::HighPass => x_1 - lp_1 * f - bp_1 * fq,
            };
            lp_2 += f * bp_2;
            bp_2 += -fq * bp_2 - f * lp_2 + y;
            if mode == Mode::BandPassNormalized {
                bp_2 += x_2;
            }
            x_2 = y;
            *sample = match mode {
                Mode::LowPass => lp_2 * f,
                Mode::BandPassNormalized => bp_2 * fq,
                Mode::HighPass => x_2 - lp_2 * f - bp_2 * fq,
            };
        }
        self.lp = [lp_1, lp_2];
        self.bp = [bp_1, bp_2];
        self.x = [x_1, x_2];
    }
}

/// PooledDelayLine over a slice of the bank's shared delay memory.
#[derive(Clone, Copy, Debug, Default)]
struct PooledDelayLine {
    offset: usize,
    size: usize,
    head: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Band {
    group: i32,
    sample_rate: f32,
    post_gain: f32,
    svf: [CrossoverSvf; 2],
    decimation_factor: usize,
    samples: usize,
    delay_line: PooledDelayLine,
    delay: i32,
}

#[derive(Clone, Copy, Debug)]
struct FilterBank {
    mid_src_down: SrcDown<MID_FACTOR, 36, 72>,
    mid_src_up: SrcUp<MID_FACTOR, 36, 12>,
    low_src_down: SrcDown<LOW_FACTOR, 48, 96>,
    low_src_up: SrcUp<LOW_FACTOR, 48, 12>,
    tmp: [[f32; MAX_FILTER_BANK_BLOCK_SIZE]; 2],
    samples: [f32; SAMPLE_MEMORY_SIZE],
    delay_buffer: [f32; DELAY_LINE_SIZE],
    band: [Band; NUM_BANDS + 1],
}

impl FilterBank {
    fn new() -> Self {
        Self {
            mid_src_down: SrcDown::new(&SRC_DOWN_3_36),
            mid_src_up: SrcUp::new(&SRC_UP_3_36),
            low_src_down: SrcDown::new(&SRC_DOWN_4_48),
            low_src_up: SrcUp::new(&SRC_UP_4_48),
            tmp: [[0.0; MAX_FILTER_BANK_BLOCK_SIZE]; 2],
            samples: [0.0; SAMPLE_MEMORY_SIZE],
            delay_buffer: [0.0; DELAY_LINE_SIZE],
            band: [Band::default(); NUM_BANDS + 1],
        }
    }

    fn clear(&mut self) {
        for buffer in &mut self.tmp {
            buffer.fill(0.0);
        }
        self.samples.fill(0.0);
        self.delay_buffer.fill(0.0);
        self.band = [Band::default(); NUM_BANDS + 1];
    }

    fn init(&mut self, sample_rate: f32) {
        self.low_src_down.init();
        self.low_src_up.init();
        self.mid_src_down.init();
        self.mid_src_up.init();

        let mut max_delay = 0i32;
        let mut samples = 0usize;
        let mut group = -1i32;
        let mut decimation_factor = -1i32;
        for (b, coefficients) in self.band.iter_mut().zip(FILTER_BANK.iter()) {
            b.decimation_factor = coefficients[0] as i32 as usize;
            if b.decimation_factor as i32 != decimation_factor {
                decimation_factor = b.decimation_factor as i32;
                group += 1;
            }
            b.group = group;
            b.sample_rate = sample_rate / b.decimation_factor as f32;
            b.samples = samples;
            samples += MAX_FILTER_BANK_BLOCK_SIZE / b.decimation_factor;
            b.delay = coefficients[1] as i32;
            b.delay *= b.decimation_factor as i32;
            b.post_gain = coefficients[2];
            max_delay = max_delay.max(b.delay);
            for pass in 0..2 {
                b.svf[pass].init();
                b.svf[pass].f = coefficients[pass * 2 + 3];
                b.svf[pass].fq = coefficients[pass * 2 + 4];
            }
        }
        assert!(samples <= SAMPLE_MEMORY_SIZE);
        self.band[NUM_BANDS].group = self.band[NUM_BANDS - 1].group + 1;
        max_delay = max_delay.min(256);
        let low_delay =
            (SrcDown::<LOW_FACTOR, 48, 96>::delay() + SrcUp::<LOW_FACTOR, 48, 12>::delay()) as i32;
        let mid_down = SrcDown::<MID_FACTOR, 36, 72>::delay() as i32;
        let mid_up = SrcUp::<MID_FACTOR, 36, 12>::delay() as i32;
        let mut delay_offset = 0usize;
        for b in self.band.iter_mut().take(NUM_BANDS) {
            let mut compensation = max_delay - b.delay;
            if b.group == 0 {
                compensation -= LOW_FACTOR as i32 * low_delay;
                compensation -= mid_down;
                compensation -= mid_up;
            } else if b.group == 1 {
                compensation -= mid_down;
                compensation -= mid_up;
            }
            compensation = (compensation - b.decimation_factor as i32 / 2).max(0);
            let delay = compensation / b.decimation_factor as i32;
            let size = delay as usize + 1;
            assert!(delay_offset + size <= DELAY_LINE_SIZE);
            self.delay_buffer[delay_offset..delay_offset + size].fill(0.0);
            b.delay_line = PooledDelayLine {
                offset: delay_offset,
                size,
                head: 0,
            };
            delay_offset += size;
        }
    }

    fn analyze(&mut self, input: &[f32]) {
        let size = input.len();
        let [tmp_0, tmp_1] = &mut self.tmp;
        self.mid_src_down
            .process(input, &mut tmp_0[..size / MID_FACTOR]);
        self.low_src_down.process(
            &tmp_0[..size / MID_FACTOR],
            &mut tmp_1[..size / MID_FACTOR / LOW_FACTOR],
        );
        for i in 0..NUM_BANDS {
            let b = &mut self.band[i];
            let band_size = size / b.decimation_factor;
            let source: &[f32] = match b.group {
                0 => &tmp_1[..],
                1 => &tmp_0[..],
                _ => input,
            };
            let destination = &mut self.samples[b.samples..b.samples + band_size];
            destination.copy_from_slice(&source[..band_size]);
            let mode = if i == 0 {
                Mode::LowPass
            } else if i == NUM_BANDS - 1 {
                Mode::HighPass
            } else {
                Mode::BandPassNormalized
            };
            for pass in 0..2 {
                b.svf[pass].process(mode, destination);
            }
            // Apply post-gain.
            let gain = b.post_gain;
            for sample in destination.iter_mut() {
                *sample *= gain;
            }
        }
    }

    // Indexes the band samples, delay memory and group buffer in lockstep.
    #[allow(clippy::needless_range_loop)]
    fn synthesize(&mut self, out: &mut [f32]) {
        let size = out.len();
        let [tmp_0, tmp_1] = &mut self.tmp;
        tmp_1[..size / self.band[0].decimation_factor].fill(0.0);
        for i in 0..NUM_BANDS {
            let b = &mut self.band[i];
            let band_size = size / b.decimation_factor;
            let s: &mut [f32] = match b.group {
                0 => &mut tmp_1[..],
                1 => &mut tmp_0[..],
                _ => &mut *out,
            };
            let line = &mut b.delay_line;
            for j in 0..band_size {
                let memory = &mut self.delay_buffer[line.offset..line.offset + line.size];
                memory[line.head] = self.samples[b.samples + j];
                line.head = (line.head + 1) % line.size;
                s[j] += memory[line.head];
            }
            let group = b.group;
            if self.band[i + 1].group != group {
                if group == 0 {
                    self.low_src_up
                        .process(&tmp_1[..band_size], &mut tmp_0[..band_size * LOW_FACTOR]);
                } else if group == 1 {
                    self.mid_src_up
                        .process(&tmp_0[..band_size], &mut out[..band_size * MID_FACTOR]);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct EnvelopeFollower {
    attack: f32,
    decay: f32,
    envelope: f32,
    peak: f32,
    freeze: bool,
}

impl EnvelopeFollower {
    fn init(&mut self) {
        self.envelope = 0.0;
        self.freeze = false;
        self.attack = 0.1;
        self.decay = 0.1;
        self.peak = 0.0;
    }

    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let follower_gain = (NUM_BANDS as f32).sqrt();
        let mut envelope = self.envelope;
        let attack = if self.freeze { 0.0 } else { self.attack };
        let decay = if self.freeze { 0.0 } else { self.decay };
        let mut peak = 0.0;
        for (sample, envelope_out) in input.iter().zip(out.iter_mut()) {
            let error = (*sample * follower_gain).abs() - envelope;
            envelope += if error > 0.0 { attack } else { decay } * error;
            if envelope > peak {
                peak = envelope;
            }
            *envelope_out = envelope;
        }
        self.envelope = envelope;
        let error = peak - self.peak;
        self.peak += if error > 0.0 { 0.5 } else { 0.1 } * error;
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct BandGain {
    carrier: f32,
    vocoder: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Vocoder {
    release_time: f32,
    formant_shift: f32,
    previous_gain: [BandGain; NUM_BANDS],
    gain: [BandGain; NUM_BANDS],
    tmp: [f32; MAX_FILTER_BANK_BLOCK_SIZE],
    modulator_filter_bank: FilterBank,
    carrier_filter_bank: FilterBank,
    limiter_peak: f32,
    follower: [EnvelopeFollower; NUM_BANDS],
}

impl Vocoder {
    pub(super) fn new() -> Self {
        Self {
            release_time: 0.0,
            formant_shift: 0.0,
            previous_gain: [BandGain::default(); NUM_BANDS],
            gain: [BandGain::default(); NUM_BANDS],
            tmp: [0.0; MAX_FILTER_BANK_BLOCK_SIZE],
            modulator_filter_bank: FilterBank::new(),
            carrier_filter_bank: FilterBank::new(),
            limiter_peak: 0.0,
            follower: [EnvelopeFollower::default(); NUM_BANDS],
        }
    }

    /// Zero every member, as static construction does, before `init`.
    pub(super) fn clear(&mut self) {
        self.release_time = 0.0;
        self.formant_shift = 0.0;
        self.previous_gain = [BandGain::default(); NUM_BANDS];
        self.gain = [BandGain::default(); NUM_BANDS];
        self.tmp.fill(0.0);
        self.modulator_filter_bank.clear();
        self.carrier_filter_bank.clear();
        self.limiter_peak = 0.0;
        self.follower = [EnvelopeFollower::default(); NUM_BANDS];
    }

    pub(super) fn init(&mut self, sample_rate: f32) {
        self.modulator_filter_bank.init(sample_rate);
        self.carrier_filter_bank.init(sample_rate);
        self.limiter_peak = 0.5;
        self.release_time = 0.5;
        self.formant_shift = 0.5;
        self.previous_gain = [BandGain::default(); NUM_BANDS];
        self.gain = [BandGain::default(); NUM_BANDS];
        for follower in &mut self.follower {
            follower.init();
        }
    }

    pub(super) fn set_release_time(&mut self, release_time: f32) {
        self.release_time = release_time;
    }

    pub(super) fn set_formant_shift(&mut self, formant_shift: f32) {
        self.formant_shift = formant_shift;
    }

    /// Source Vocoder::Process with `carrier` and `out` as one buffer, as
    /// warps::Modulator calls it: the carrier is fully analyzed before
    /// synthesis overwrites it.
    pub(super) fn process(
        &mut self,
        modulator: &[f32],
        carrier_out: &mut [f32],
        guarded: &mut u32,
    ) {
        let size = carrier_out.len();
        // Run through filter banks.
        self.modulator_filter_bank.analyze(modulator);
        self.carrier_filter_bank.analyze(carrier_out);

        // Set the attack/release time of envelope followers.
        let mut f = 80.0 * semitones_to_ratio(-72.0 * self.release_time, guarded);
        for (i, follower) in self.follower.iter_mut().enumerate() {
            let decay = f / self.modulator_filter_bank.band[i].sample_rate;
            follower.attack = decay * 2.0;
            follower.decay = decay * 0.5;
            follower.freeze = self.release_time > 0.995;
            f *= 1.2599; // 2 ** (4/12.0), a third octave.
        }

        // Compute the amplitude (or modulation amount) in all bands.
        let mut formant_shift_amount = 2.0 * (self.formant_shift - 0.5).abs();
        formant_shift_amount *= 2.0 - formant_shift_amount;
        formant_shift_amount *= 2.0 - formant_shift_amount;
        let envelope_increment = 4.0 * semitones_to_ratio(-48.0 * self.formant_shift, guarded);
        let mut envelope = 0.0;
        let last_band: f32 = NUM_BANDS as f32 - 1.0001;
        for gain in self.gain.iter_mut() {
            let source_band = constrain(envelope, 0.0, last_band);
            let source_band_integral = source_band as i32 as usize;
            let source_band_fractional = source_band - source_band_integral as f32;
            let a = self.follower[source_band_integral].peak;
            let b = self.follower[source_band_integral + 1].peak;
            let mut band_gain = a + (b - a) * source_band_fractional;
            let attenuation = envelope - last_band;
            if attenuation >= 0.0 {
                band_gain *= 1.0 / (1.0 + 1.0 * attenuation);
            }
            envelope += envelope_increment;
            gain.carrier = band_gain * formant_shift_amount;
            gain.vocoder = 1.0 - formant_shift_amount;
        }

        for i in 0..NUM_BANDS {
            let modulator_band = self.modulator_filter_bank.band[i];
            let carrier_band = self.carrier_filter_bank.band[i];
            let band_size = size / modulator_band.decimation_factor;
            let step = 1.0 / band_size as f32;
            let envelope = &mut self.tmp[..band_size];
            self.follower[i].process(
                &self.modulator_filter_bank.samples
                    [modulator_band.samples..modulator_band.samples + band_size],
                envelope,
            );
            let carrier = &mut self.carrier_filter_bank.samples
                [carrier_band.samples..carrier_band.samples + band_size];
            let mut vocoder_gain = self.previous_gain[i].vocoder;
            let vocoder_gain_increment = (self.gain[i].vocoder - vocoder_gain) * step;
            let mut carrier_gain = self.previous_gain[i].carrier;
            let carrier_gain_increment = (self.gain[i].carrier - carrier_gain) * step;
            for (sample, e) in carrier.iter_mut().zip(envelope.iter()) {
                *sample *= carrier_gain + vocoder_gain * *e;
                vocoder_gain += vocoder_gain_increment;
                carrier_gain += carrier_gain_increment;
            }
            self.previous_gain[i] = self.gain[i];
        }

        self.carrier_filter_bank.synthesize(carrier_out);

        // Limiter::Process(out, 1.4f, size).
        for sample in carrier_out.iter_mut() {
            let s = *sample * 1.4;
            let error = s.abs() - self.limiter_peak;
            self.limiter_peak += if error > 0.0 { 0.05 } else { 0.00002 } * error;
            let gain = if self.limiter_peak <= 1.0 {
                1.0
            } else {
                1.0 / self.limiter_peak
            };
            *sample = soft_limit(s * gain * 0.8);
        }
    }
}
