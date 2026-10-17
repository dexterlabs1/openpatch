// Preserve the pinned source arithmetic/decimal constants and update order.
#![allow(clippy::manual_memcpy)]

// Copyright 2012 Emilie Gillet.
// Copyright 2015 Emilie Gillet.
// Copyright 2017 Emilie Gillet.
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
// Rust adaptation of the portable Tides 2018 PolySlopeGenerator kernel.
// eurorack f4cfa7697095a28d292bff758beb51deb49b5a86;
// stmlib e3bd7c9cc00e4364166f9905c0509b6ffd0535ec.
// Source file hashes are pinned by scripts/tides2_reference/source_manifest.json.
// tides2/poly_slope_generator.h:
// d871990e563b700a76ea5ff24516864fe21f3886331fec7538595af5d06b39f9
// tides2/poly_slope_generator.cc:
// 3765745bd26b80469cd9ee7fca93c1a339b8e719ec626a9f005a439d1d07610a
// stmlib/dsp/dsp.h:
// 98b42900c4accbe9fbb2cfe2ec6fada024b57ec9a1becff008b0f35bd4fd8ddb
// stmlib/dsp/parameter_interpolator.h:
// 4b05028c320e6946418a36e9662a4e5a142e5bfab0423835a77d074d9c7fc217
// stmlib/dsp/polyblep.h:
// b36025a8a9b2e9a37156e45cfdee937451f788529089b51b9f490da1a5b0063b
// stmlib/dsp/hysteresis_quantizer.h:
// 06032c1ec2e21cb7e904544f839a2a59b4c4a9f3f391918e6adfac7f68ecc243
// Arithmetic order follows the C++ TEST domain (-ffp-contract=off).
// Reset intentionally clears only the two-pole smoothing filter.

#[path = "kernel/ramp.rs"]
mod ramp;
#[path = "kernel/ratios.rs"]
mod ratios;
#[path = "kernel/resources.rs"]
mod resources;
#[path = "kernel/shaper.rs"]
mod shaper;

use ramp::RampGenerator;
use ratios::{AUDIO_RATIO_TABLE, CONTROL_RATIO_TABLE};
use resources::{LUT_BIPOLAR_FOLD, LUT_UNIPOLAR_FOLD};
use shaper::{RampShaper, RampWaveshaper};

#[derive(Clone, Copy, Debug)]
struct Ratio {
    ratio: f32,
    q: i32,
}
impl Ratio {
    const ONE: Self = Self { ratio: 1.0, q: 1 };
}

// std::min/max and CONSTRAIN ordering, including inverted bounds, from stmlib.
fn cmin(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}
fn cmax(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}
fn constrain(value: &mut f32, min: f32, max: f32) {
    if *value < min {
        *value = min;
    } else if *value > max {
        *value = max;
    }
}
fn interpolate(table: &[f32], mut index: f32) -> f32 {
    index *= 1024.0;
    let integral = index as i32;
    let fractional = index - integral as f32;
    let a = table[integral as usize];
    let b = table[integral as usize + 1];
    a + (b - a) * fractional
}
fn tame(mut f0: f32, harmonics: f32, order: f32) -> f32 {
    f0 *= harmonics;
    let max_f = 0.5 * (1.0 / order);
    let mut amount = 1.0 - (f0 - max_f) / (0.5 - max_f);
    constrain(&mut amount, 0.0, 1.0);
    amount * amount * amount
}
fn fold(mode: u8, unipolar: f32, amount: f32) -> f32 {
    if mode == 1 {
        let bipolar = 2.0 * unipolar - 1.0;
        let folded = if amount > 0.0 {
            interpolate(&LUT_BIPOLAR_FOLD, 0.5 + bipolar * (0.03 + 0.46 * amount))
        } else {
            0.0
        };
        5.0 * (bipolar + (folded - bipolar) * amount)
    } else {
        let folded = if amount > 0.0 {
            interpolate(&LUT_UNIPOLAR_FOLD, unipolar * amount)
        } else {
            0.0
        };
        8.0 * (unipolar + (folded - unipolar) * amount)
    }
}

struct Interpolator {
    value: f32,
    increment: f32,
}
impl Interpolator {
    fn new(state: f32, target: f32, size: usize) -> Self {
        Self {
            value: state,
            increment: (target - state) / size as f32,
        }
    }
    fn next(&mut self) -> f32 {
        self.value += self.increment;
        self.value
    }
}

/// Four related generators in the pinned source's portable processing domain.
/// Inputs are normalized kernel controls; gates contain upstream GateFlags bits.
#[derive(Clone, Debug)]
pub(crate) struct TidalKernel {
    frequency: f32,
    pw: f32,
    shift: f32,
    shape: f32,
    fold: f32,
    ratio_quantized: i32,
    ramp: RampGenerator,
    shaper: [RampShaper; 4],
    waveshaper: [RampWaveshaper; 4],
    lp1: [f32; 4],
    lp2: [f32; 4],
}

impl TidalKernel {
    pub(crate) fn new() -> Self {
        // Matches zero-initialized firmware storage followed by Init().
        Self {
            frequency: 0.01,
            pw: 0.0,
            shift: 0.0,
            shape: 0.0,
            fold: 0.0,
            ratio_quantized: 0,
            ramp: RampGenerator::new(),
            shaper: [RampShaper::default(); 4],
            waveshaper: [RampWaveshaper::default(); 4],
            lp1: [0.0; 4],
            lp2: [0.0; 4],
        }
    }

    pub(crate) fn reset_filter(&mut self) {
        self.lp1.fill(0.0);
        self.lp2.fill(0.0);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        ramp_mode: u8,
        output_mode: u8,
        range: u8,
        mut frequency: f32,
        mut slope: f32,
        mut shape: f32,
        mut smoothness: f32,
        shift: f32,
        gates: &[u8],
        external_ramp: Option<&[f32]>,
        output: &mut [[f32; 4]],
    ) {
        let size = output.len();
        assert!(ramp_mode < 3 && output_mode < 4 && range < 2);
        assert!(gates.len() >= size);
        assert!(external_ramp.is_none_or(|ramp| ramp.len() >= size));
        if size == 0 {
            return;
        }
        frequency = cmin(frequency, 0.25);
        if range == 0 && slope < 0.5 {
            slope = 0.5 + 0.6 * (slope - 0.5) / ((slope - 0.5).abs() + 0.1);
        }
        if external_ramp.is_some() && ramp_mode == 2 {
            frequency *= 1.0 + 2.0 * (slope - 0.5).abs();
        }
        let harmonics = 3.0 + (slope - 0.5).abs() * 5.0;
        let shape_amount = (shape - 0.5).abs() * 2.0;
        let attenuation = tame(frequency, harmonics, 16.0);
        shape = 0.5 + (shape - 0.5) * attenuation;
        if smoothness > 0.5 {
            smoothness = 0.5
                + (smoothness - 0.5)
                    * tame(
                        frequency,
                        harmonics * (3.0 + shape_amount * attenuation * 5.0),
                        12.0,
                    );
        }
        self.render_internal(
            ramp_mode,
            output_mode,
            range,
            frequency,
            slope,
            shape,
            smoothness,
            shift,
            gates,
            external_ramp,
            output,
        );
        if smoothness < 0.5 {
            let mut ratio = smoothness * 2.0;
            ratio *= ratio;
            ratio *= ratio;
            let channels = if output_mode == 0 { 1 } else { 4 };
            let mut f = [0.0; 4];
            for (i, f) in f.iter_mut().enumerate().take(channels) {
                let source = if output_mode == 3 { i } else { 0 };
                *f = self.ramp.frequency[source] * 0.5;
                *f += (1.0 - *f) * ratio;
            }
            for sample in output {
                for i in 0..channels {
                    self.lp1[i] += f[i] * (sample[i] - self.lp1[i]);
                    self.lp2[i] += f[i] * (self.lp1[i] - self.lp2[i]);
                    sample[i] = self.lp2[i];
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_internal(
        &mut self,
        mode: u8,
        output_mode: u8,
        range: u8,
        frequency: f32,
        pw: f32,
        shape: f32,
        smoothness: f32,
        shift: f32,
        gates: &[u8],
        external_ramp: Option<&[f32]>,
        output: &mut [[f32; 4]],
    ) {
        let is_phasor = !(range == 1 && mode == 1);
        let size = output.len();
        let mut fm = Interpolator::new(self.frequency, frequency, size);
        let mut pwm = Interpolator::new(self.pw, pw, size);
        let mut shift_mod = Interpolator::new(self.shift, 2.0 * shift - 1.0, size);
        let mut shape_mod = Interpolator::new(
            self.shape,
            if is_phasor {
                shape * 5.9999 + 5.0
            } else {
                shape * 3.9999
            },
            size,
        );
        let mut fold_mod = Interpolator::new(self.fold, cmax(2.0 * (smoothness - 0.5), 0.0), size);
        if output_mode == 3 {
            let mut value = shift;
            value *= 21.0;
            value += -0.5;
            value += 0.0;
            let sign = if value > self.ratio_quantized as f32 {
                -1.0
            } else {
                1.0
            };
            let mut q = (value + sign * 0.05 + 0.5) as i32;
            q = q.clamp(0, 20);
            self.ratio_quantized = q;
            self.ramp.next_ratio = if range == 0 {
                CONTROL_RATIO_TABLE[q as usize]
            } else {
                AUDIO_RATIO_TABLE[q as usize]
            };
        }
        for (i, sample) in output.iter_mut().enumerate() {
            let f0 = fm.next();
            let pw = pwm.next();
            let shift = shift_mod.next();
            let step = shift * (1.0 / 3.0);
            let partial_step = shift * (1.0 / 4.0);
            let fold_amount = fold_mod.next();
            let pw_increment = (if shift > 0.0 { 1.0 - pw } else { pw }) * step;
            let per_channel_pw = std::array::from_fn::<_, 4, _>(|j| pw + pw_increment * j as f32);
            let ramp = external_ramp.map(|r| r[i]);
            let gate = if ramp.is_some() { 0 } else { gates[i] };
            if output_mode == 2 && mode == 2 {
                self.ramp
                    .step(mode, output_mode, range, f0, &per_channel_pw, gate, ramp);
            } else {
                self.ramp
                    .step(mode, output_mode, range, f0, &[pw], gate, ramp);
            }
            let shape = shape_mod.next();
            let shape_integral = shape as i32;
            let shape_fractional = shape - shape_integral as f32;
            let table = shape_integral as usize * 1025;
            if output_mode == 0 {
                let phase = self.ramp.phase[0];
                let frequency = self.ramp.frequency[0];
                let raw = self.shaper[0].slope(mode, range, phase, 0.0, frequency, pw);
                let slope = self.waveshaper[0].shape(mode, raw, table, shape_fractional);
                sample[0] = fold(mode, slope, fold_amount) * shift;
                let scaled = if is_phasor {
                    self.waveshaper[1].shape(mode, raw, 8200, 0.0)
                } else {
                    raw
                };
                sample[1] = if mode == 1 {
                    10.0 * scaled - 5.0
                } else {
                    8.0 * scaled
                };
                sample[2] = self.shaper[2].eoa(mode, range, phase, frequency, pw) * 8.0;
                sample[3] = self.shaper[3].eor(mode, range, phase, frequency) * 8.0;
            } else if output_mode == 1 {
                let raw = self.shaper[0].slope(
                    mode,
                    range,
                    self.ramp.phase[0],
                    0.0,
                    self.ramp.frequency[0],
                    pw,
                );
                let shaped = self.waveshaper[0].shape(mode, raw, table, shape_fractional);
                let slope = fold(mode, shaped, fold_amount) * if shift < 0.0 { -1.0 } else { 1.0 };
                let channel_index = (shift * 5.1).abs();
                for (j, output) in sample.iter_mut().enumerate() {
                    let channel = (j + 1) as f32;
                    let gain = cmax(1.0 - (channel - channel_index).abs(), 0.0);
                    *output = slope * gain * if range == 1 { 2.0 - gain } else { 1.0 };
                }
            } else if output_mode == 2 {
                let mut phase_shift = 0.0;
                for (j, output) in sample.iter_mut().enumerate() {
                    let source = if mode == 2 { j } else { 0 };
                    let raw = self.shaper[j].slope(
                        mode,
                        range,
                        self.ramp.phase[source],
                        phase_shift,
                        self.ramp.frequency[source],
                        if mode == 0 { per_channel_pw[j] } else { pw },
                    );
                    let shaped = self.waveshaper[j].shape(mode, raw, table, shape_fractional);
                    *output = fold(mode, shaped, fold_amount);
                    phase_shift -= if range == 1 { step } else { partial_step };
                }
            } else {
                for (j, output) in sample.iter_mut().enumerate() {
                    let raw = self.shaper[j].slope(
                        mode,
                        range,
                        self.ramp.phase[j],
                        0.0,
                        self.ramp.frequency[j],
                        pw,
                    );
                    let shaped = self.waveshaper[j].shape(mode, raw, table, shape_fractional);
                    *output = fold(mode, shaped, fold_amount);
                }
            }
        }
        self.frequency = fm.value;
        self.pw = pwm.value;
        self.shift = shift_mod.value;
        self.shape = shape_mod.value;
        self.fold = fold_mod.value;
    }
}
