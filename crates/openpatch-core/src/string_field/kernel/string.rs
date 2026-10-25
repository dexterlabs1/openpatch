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
// Rust adaptation (OpenPatch String Field) of rings/dsp/string.h/.cc at
// eurorack f4cfa7697095a28d292bff758beb51deb49b5a86: a Karplus-Strong string
// with FIR/IIR damping, an optional dispersion all-pass and curved bridge, and
// a comb-filter pickup. The two delay lines live in kernel-owned storage.

use super::resources::LUT_SVF_SHIFT;
use super::stmlib::{
    crossfade, interpolate, max, min, semitones_to_ratio, DcBlocker, DelayCursor, Guard,
    ParameterInterpolator, Random, Svf,
};
use super::KERNEL_SAMPLE_RATE;

pub(super) const DELAY_LINE_SIZE: usize = 2048;
pub(super) const STIFFNESS_LINE_SIZE: usize = DELAY_LINE_SIZE / 2;

#[derive(Clone, Copy, Debug)]
struct DampingFilter {
    x: f32,
    x_: f32,
    brightness: f32,
    brightness_increment: f32,
    damping: f32,
    damping_increment: f32,
}

impl DampingFilter {
    const ZERO: Self = Self {
        x: 0.0,
        x_: 0.0,
        brightness: 0.0,
        brightness_increment: 0.0,
        damping: 0.0,
        damping_increment: 0.0,
    };

    fn init(&mut self) {
        *self = Self::ZERO;
    }

    fn configure(&mut self, damping: f32, brightness: f32, size: usize) {
        if size == 0 {
            self.damping = damping;
            self.brightness = brightness;
            self.damping_increment = 0.0;
            self.brightness_increment = 0.0;
        } else {
            let step = 1.0 / size as f32;
            self.damping_increment = (damping - self.damping) * step;
            self.brightness_increment = (brightness - self.brightness) * step;
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let h0 = (1.0 + self.brightness) * 0.5;
        let h1 = (1.0 - self.brightness) * 0.25;
        let y = self.damping * (h0 * self.x + h1 * (x + self.x_));
        self.x_ = self.x;
        self.x = x;
        self.brightness += self.brightness_increment;
        self.damping += self.damping_increment;
        y
    }
}

/// Storage of one string's main and stiffness delay lines.
pub(super) struct StringLines<'a> {
    pub(super) string: &'a mut [f32; DELAY_LINE_SIZE],
    pub(super) stretch: &'a mut [f32; STIFFNESS_LINE_SIZE],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct KarplusString {
    frequency: f32,
    dispersion: f32,
    brightness: f32,
    damping: f32,
    position: f32,
    delay: f32,
    clamped_position: f32,
    previous_dispersion: f32,
    previous_damping_compensation: f32,
    enable_dispersion: bool,
    dispersion_noise: f32,
    src_phase: f32,
    out_sample: [f32; 2],
    aux_sample: [f32; 2],
    curved_bridge: f32,
    string: DelayCursor<DELAY_LINE_SIZE>,
    stretch: DelayCursor<STIFFNESS_LINE_SIZE>,
    fir_damping_filter: DampingFilter,
    iir_damping_filter: Svf,
    dc_blocker: DcBlocker,
}

impl KarplusString {
    pub(super) const ZERO: Self = Self {
        frequency: 0.0,
        dispersion: 0.0,
        brightness: 0.0,
        damping: 0.0,
        position: 0.0,
        delay: 0.0,
        clamped_position: 0.0,
        previous_dispersion: 0.0,
        previous_damping_compensation: 0.0,
        enable_dispersion: false,
        dispersion_noise: 0.0,
        src_phase: 0.0,
        out_sample: [0.0; 2],
        aux_sample: [0.0; 2],
        curved_bridge: 0.0,
        string: DelayCursor::ZERO,
        stretch: DelayCursor::ZERO,
        fir_damping_filter: DampingFilter::ZERO,
        iir_damping_filter: Svf::ZERO,
        dc_blocker: DcBlocker::ZERO,
    };

    /// `String::Init`; `src_phase_` is left as it was, as in the source.
    pub(super) fn init(&mut self, enable_dispersion: bool, lines: StringLines<'_>) {
        self.enable_dispersion = enable_dispersion;
        self.string.reset(lines.string);
        self.stretch.reset(lines.stretch);
        self.fir_damping_filter.init();
        self.iir_damping_filter.init();
        self.frequency = 220.0 / KERNEL_SAMPLE_RATE;
        self.dispersion = 0.25;
        self.brightness = 0.5;
        self.damping = 0.3;
        self.position = 0.8;
        self.delay = 1.0 / self.frequency;
        self.clamped_position = 0.0;
        self.previous_dispersion = 0.0;
        self.dispersion_noise = 0.0;
        self.curved_bridge = 0.0;
        self.previous_damping_compensation = 0.0;
        self.out_sample = [0.0; 2];
        self.aux_sample = [0.0; 2];
        self.dc_blocker.init(1.0 - 20.0 / KERNEL_SAMPLE_RATE);
    }

    /// `set_frequency(frequency, coefficient)`: glide toward the target.
    pub(super) fn set_frequency(&mut self, frequency: f32, coefficient: f32) {
        self.frequency += coefficient * (frequency - self.frequency);
    }

    pub(super) fn set_dispersion(&mut self, dispersion: f32) {
        self.dispersion = dispersion;
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

    #[allow(clippy::too_many_arguments)]
    pub(super) fn process(
        &mut self,
        lines: StringLines<'_>,
        input: &[f32],
        out: &mut [f32],
        aux: &mut [f32],
        random: &mut Random,
        guard: &Guard,
    ) {
        let size = input.len();
        let mut delay = 1.0 / self.frequency;
        // CONSTRAIN(delay, 4.0f, kDelayLineSize - 4.0f)
        if delay < 4.0 {
            delay = 4.0;
        } else if delay > DELAY_LINE_SIZE as f32 - 4.0 {
            delay = DELAY_LINE_SIZE as f32 - 4.0;
        }
        // If there is not enough delay time in the delay line, the string
        // plays at the lowest possible note and is upsampled on the fly with
        // a linear interpolator (f0 < 11.7 Hz).
        let mut src_ratio = delay * self.frequency;
        if src_ratio >= 0.9999 {
            self.src_phase = 1.0;
            src_ratio = 1.0;
        }
        let clamped_position = 0.5 - 0.98 * (self.position - 0.5).abs();

        // Linearly interpolate all comb-related CV parameters for each sample.
        let mut delay_modulation = ParameterInterpolator::new(self.delay, delay, size);
        let mut position_modulation =
            ParameterInterpolator::new(self.clamped_position, clamped_position, size);
        let mut dispersion_modulation =
            ParameterInterpolator::new(self.previous_dispersion, self.dispersion, size);

        // For damping/absorption, the interpolation is done in the filter.
        let lf_damping = self.damping * (2.0 - self.damping);
        let rt60 = 0.07 * semitones_to_ratio(lf_damping * 96.0, guard) * KERNEL_SAMPLE_RATE;
        let rt60_base_2_12 = max(-120.0 * delay / src_ratio / rt60, -127.0);
        let mut damping_coefficient = semitones_to_ratio(rt60_base_2_12, guard);
        let mut brightness = self.brightness * self.brightness;
        let noise_filter = semitones_to_ratio((self.brightness - 1.0) * 48.0, guard);
        let mut damping_cutoff = min(
            24.0 + self.damping * self.damping * 48.0 + self.brightness * self.brightness * 24.0,
            84.0,
        );
        let mut damping_f = min(
            self.frequency * semitones_to_ratio(damping_cutoff, guard),
            0.499,
        );

        // Crossfade to infinite decay.
        if self.damping >= 0.95 {
            let to_infinite = 20.0 * (self.damping - 0.95);
            damping_coefficient += to_infinite * (1.0 - damping_coefficient);
            brightness += to_infinite * (1.0 - brightness);
            damping_f += to_infinite * (0.4999 - damping_f);
            damping_cutoff += to_infinite * (128.0 - damping_cutoff);
        }

        self.fir_damping_filter
            .configure(damping_coefficient, brightness, size);
        self.iir_damping_filter.set_f_q_accurate(damping_f, 0.5);
        let mut damping_compensation_modulation = ParameterInterpolator::new(
            self.previous_damping_compensation,
            1.0 - interpolate(&LUT_SVF_SHIFT, damping_cutoff, 1.0, guard),
            size,
        );

        for ((x, o), a) in input.iter().zip(out.iter_mut()).zip(aux.iter_mut()) {
            self.src_phase += src_ratio;
            if self.src_phase > 1.0 {
                self.src_phase -= 1.0;

                let mut delay = delay_modulation.next();
                let comb_delay = delay * position_modulation.next();
                delay *= damping_compensation_modulation.next(); // IIR delay.
                delay -= 1.0; // FIR delay.

                let mut s;
                if self.enable_dispersion {
                    let mut noise = 2.0 * random.get_float() - 1.0;
                    noise *= 1.0 / (0.2 + noise_filter);
                    self.dispersion_noise += noise_filter * (noise - self.dispersion_noise);
                    let dispersion = dispersion_modulation.next();
                    let stretch_point = if dispersion <= 0.0 {
                        0.0
                    } else {
                        dispersion * (2.0 - dispersion) * 0.475
                    };
                    let mut noise_amount = if dispersion > 0.75 {
                        4.0 * (dispersion - 0.75)
                    } else {
                        0.0
                    };
                    let mut bridge_curving = if dispersion < 0.0 { -dispersion } else { 0.0 };

                    noise_amount = noise_amount * noise_amount * 0.025;
                    let ac_blocking_amount = bridge_curving;
                    bridge_curving = bridge_curving * bridge_curving * 0.01;
                    let ap_gain = -0.618 * dispersion / (0.15 + dispersion.abs());

                    let mut delay_fm = 1.0f32;
                    delay_fm += self.dispersion_noise * noise_amount;
                    delay_fm -= self.curved_bridge * bridge_curving;
                    delay *= delay_fm;

                    let ap_delay = delay * stretch_point;
                    let main_delay = delay - ap_delay;
                    if ap_delay >= 4.0 && main_delay >= 4.0 {
                        s = self.string.read_hermite(lines.string, main_delay);
                        s = self
                            .stretch
                            .allpass(lines.stretch, s, ap_delay as usize, ap_gain);
                    } else {
                        s = self.string.read_hermite(lines.string, delay);
                    }
                    let mut s_ac = [s];
                    self.dc_blocker.process(&mut s_ac);
                    s += ac_blocking_amount * (s_ac[0] - s);

                    let value = s.abs() - 0.025;
                    let sign = if s > 0.0 { 1.0 } else { -1.5 };
                    self.curved_bridge = (value.abs() + value) * sign;
                } else {
                    s = self.string.read_hermite(lines.string, delay);
                }

                s += *x; // When f0 < 11.7 Hz, causes ugly bitcrushing on the input!
                s = self.fir_damping_filter.process(s);
                s = self.iir_damping_filter.process_low_pass(s);
                self.string.write(lines.string, s);
                self.out_sample[1] = self.out_sample[0];
                self.aux_sample[1] = self.aux_sample[0];
                self.out_sample[0] = s;
                self.aux_sample[0] = self.string.read(lines.string, comb_delay);
            }
            *o += crossfade(self.out_sample[1], self.out_sample[0], self.src_phase);
            *a += crossfade(self.aux_sample[1], self.aux_sample[0], self.src_phase);
        }
        self.delay = delay_modulation.value;
        self.clamped_position = position_modulation.value;
        self.previous_dispersion = dispersion_modulation.value;
        self.previous_damping_compensation = damping_compensation_modulation.value;
    }
}
