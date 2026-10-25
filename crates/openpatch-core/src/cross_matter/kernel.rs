// Preserve the pinned source arithmetic/decimal constants and update order.
#![allow(clippy::excessive_precision, clippy::too_many_arguments)]

// Copyright 2012 Emilie Gillet.
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
// Rust adaptation of the portable Warps warps::Modulator (cross-modulation,
// vocoder and frequency-shifter paths) with its oscillator, filter bank,
// sample-rate converters and stmlib helpers.
// eurorack f4cfa7697095a28d292bff758beb51deb49b5a86;
// stmlib e3bd7c9cc00e4364166f9905c0509b6ffd0535ec.
// Source file hashes are pinned by scripts/warps_reference/source_manifest.json:
// warps/dsp/modulator.cc, modulator.h, parameters.h, oscillator.cc/.h,
// vocoder.cc/.h, filter_bank.cc/.h, limiter.h, quadrature_oscillator.h,
// quadrature_transform.h, sample_rate_converter.h,
// sample_rate_conversion_filters.h, warps/resources.cc; stmlib dsp.h,
// filter.h, parameter_interpolator.h, units.h/.cc, utils/random.h/.cc.
//
// Adaptation notes (behaviour outside these notes follows the source):
// - Instances are value-initialized to zero, matching the firmware's static
//   storage; members the source never initializes therefore start at zero.
// - The process-global stmlib::Random state is owned per instance, seeded with
//   the source value 0x21.
// - Table reads whose index would leave a table (undefined behaviour in the
//   source) are clamped to the table and counted in `guarded_reads`.
// - Blocks are fixed at 12..=60 frames in multiples of 12, so only the
//   source's circular-buffer down-sampling branch is reachable.

#[path = "kernel/oscillator.rs"]
mod oscillator;
#[path = "kernel/resources.rs"]
mod resources;
#[path = "kernel/sample_rate.rs"]
mod sample_rate;
#[path = "kernel/vocoder.rs"]
mod vocoder;

use oscillator::{Oscillator, QuadratureOscillator};
use resources::{LUT_AP_POLES, LUT_BIPOLAR_FOLD, LUT_SIN, LUT_XFADE_IN, LUT_XFADE_OUT};
use sample_rate::{QuadratureTransform, SrcDown, SrcUp};
use vocoder::Vocoder;

pub(crate) const KERNEL_SAMPLE_RATE: f32 = 96000.0;
pub(crate) const BLOCK: usize = 60;
const MAX_BLOCK: usize = 96;
const OVERSAMPLING: usize = 6;

/// Source `Parameters`, set by the caller before each block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Parameters {
    pub(crate) channel_drive: [f32; 2],
    pub(crate) modulation_algorithm: f32,
    pub(crate) modulation_parameter: f32,
    pub(crate) frequency_shift_pot: f32,
    pub(crate) frequency_shift_cv: f32,
    pub(crate) phase_shift: f32,
    pub(crate) note: f32,
    /// 0 = external carrier, 1..=3 = internal sine/triangle/saw family.
    pub(crate) carrier_shape: i32,
}

impl Parameters {
    // Source quirk retained: modulation_algorithm is 0..1, so the 1..5
    // branches are unreachable and the skew equals the algorithm value.
    fn skewed_modulation_parameter(&self) -> f32 {
        let mut skew = 0.0;
        if self.modulation_algorithm <= 1.0 {
            skew = self.modulation_algorithm;
        } else if self.modulation_algorithm >= 5.0 {
            skew = 1.0;
        } else if self.modulation_algorithm >= 4.0 {
            skew = 5.0 - self.modulation_algorithm;
        }
        self.modulation_parameter * (1.0 + skew * (self.modulation_parameter - 1.0))
    }
}

/// Response curve of the Warps algorithm knob: warps/resources.cc
/// `lut_pot_curve`, read as warps/cv_scaler.cc `UnwrapPot` does. The input is
/// constrained to 0..1 first, so the read never leaves the table.
pub(crate) fn unwrap_pot(x: f32) -> f32 {
    let mut unused = 0;
    interpolate(
        &resources::LUT_POT_CURVE,
        constrain(x, 0.0, 1.0),
        512.0,
        &mut unused,
    )
}

// ---------------------------------------------------------------------------
// stmlib helpers

#[inline]
pub(super) fn soft_limit(x: f32) -> f32 {
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

#[inline]
pub(super) fn soft_clip(x: f32) -> f32 {
    if x < -3.0 {
        -1.0
    } else if x > 3.0 {
        1.0
    } else {
        soft_limit(x)
    }
}

#[inline]
pub(super) fn clip16(x: i32) -> i32 {
    x.clamp(-32768, 32767)
}

#[inline]
pub(super) fn constrain(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Source `Interpolate(table + base, index, size)`. Truncation toward zero
/// and a negative fractional part are preserved for negative indices.
#[inline]
pub(super) fn interpolate_at(
    table: &[f32],
    base: isize,
    index: f32,
    size: f32,
    guarded: &mut u32,
) -> f32 {
    let index = index * size;
    let integral = index as i32;
    let fractional = index - integral as f32;
    let last = table.len() as isize - 1;
    let position = base + integral as isize;
    if position < 0 || position >= last {
        // Index == table end with a zero fraction reads one element past the
        // array in the source but contributes nothing; only count real escapes.
        let at = position.clamp(0, last) as usize;
        if !(position == last && fractional == 0.0) {
            *guarded = guarded.saturating_add(1);
        }
        return table[at];
    }
    let a = table[position as usize];
    let b = table[position as usize + 1];
    a + (b - a) * fractional
}

#[inline]
pub(super) fn interpolate(table: &[f32], index: f32, size: f32, guarded: &mut u32) -> f32 {
    interpolate_at(table, 0, index, size, guarded)
}

pub(super) fn semitones_to_ratio(semitones: f32, guarded: &mut u32) -> f32 {
    let pitch = semitones + 128.0;
    let integral = pitch as i32;
    let fractional = pitch - integral as f32;
    let low = (fractional * 256.0) as i32;
    let high_index = integral.clamp(0, 255);
    let low_index = low.clamp(0, 255);
    if high_index != integral || low_index != low {
        *guarded = guarded.saturating_add(1);
    }
    resources::LUT_PITCH_RATIO_HIGH[high_index as usize]
        * resources::LUT_PITCH_RATIO_LOW[low_index as usize]
}

/// stmlib::ParameterInterpolator. The source writes the accumulated value back
/// to its state in the destructor; callers call `value()` at that point.
#[derive(Clone, Copy, Debug)]
pub(super) struct Interpolator {
    value: f32,
    increment: f32,
}

impl Interpolator {
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
    #[inline]
    pub(super) fn value(&self) -> f32 {
        self.value
    }
}

// ---------------------------------------------------------------------------
// SaturatingAmplifier (modulator.h)

#[derive(Clone, Copy, Debug, Default)]
struct SaturatingAmplifier {
    level: f32,
    drive: f32,
    post_gain: f32,
    pre_gain: f32,
}

impl SaturatingAmplifier {
    fn init(&mut self) {
        // The source assigns only drive_; other members keep static zeroes.
        self.drive = 0.0;
    }

    fn process(
        &mut self,
        drive: f32,
        limit: f32,
        input: &[[i16; 2]],
        channel: usize,
        out: &mut [f32],
        out_raw: &mut [f32],
    ) {
        let size = input.len();
        let mut drive_modulation = Interpolator::new(self.drive, drive, size);
        let mut level = self.level;
        for i in 0..size {
            let mut s = input[i][channel] as f32 / 32768.0;
            let error = s * s - level;
            level += error * if error > 0.0 { 0.1 } else { 0.0001 };
            s *= if level <= 0.0001 {
                (1.0 / 0.0001) * level
            } else {
                1.0
            };
            out[i] = s;
            out_raw[i] += s * drive_modulation.next();
        }
        self.level = level;

        let drive_2 = drive * drive;
        let pre_gain_a = drive * 0.5;
        let pre_gain_b = drive_2 * drive_2 * drive * 24.0;
        let pre_gain = pre_gain_a + (pre_gain_b - pre_gain_a) * drive_2;
        let drive_squished = drive * (2.0 - drive);
        let post_gain = 1.0 / soft_clip(0.33 + drive_squished * (pre_gain - 0.33));
        let mut pre_gain_modulation = Interpolator::new(self.pre_gain, pre_gain, size);
        let mut post_gain_modulation = Interpolator::new(self.post_gain, post_gain, size);
        for sample in out.iter_mut().take(size) {
            let pre = pre_gain_modulation.next() * *sample;
            let post = soft_clip(pre) * post_gain_modulation.next();
            *sample = pre + (post - pre) * limit;
        }
        self.drive = drive_modulation.value();
        self.pre_gain = pre_gain_modulation.value();
        self.post_gain = post_gain_modulation.value();
    }
}

// ---------------------------------------------------------------------------
// Cross-modulation algorithms (modulator.cc)

const XFADE: u8 = 0;
const FOLD: u8 = 1;
const ANALOG_RING: u8 = 2;
const DIGITAL_RING: u8 = 3;
const XOR: u8 = 4;
const COMPARATOR: u8 = 5;
const NOP: u8 = 6;

#[inline]
fn diode(x: f32) -> f32 {
    // Approximation of diode non-linearity from Julian Parker, "A simple
    // Digital model of the diode-based ring-modulator", Proc. DAFx-11.
    let sign = if x > 0.0 { 1.0 } else { -1.0 };
    let mut dead_zone = x.abs() - 0.667;
    dead_zone += dead_zone.abs();
    dead_zone *= dead_zone;
    0.04324765822726063 * dead_zone * sign
}

#[inline(always)]
fn xmod<const ALGORITHM: u8>(x_1: f32, x_2: f32, parameter: f32, guarded: &mut u32) -> f32 {
    match ALGORITHM {
        XFADE => {
            let fade_in = interpolate(&LUT_XFADE_IN, parameter, 256.0, guarded);
            let fade_out = interpolate(&LUT_XFADE_OUT, parameter, 256.0, guarded);
            x_1 * fade_in + x_2 * fade_out
        }
        FOLD => {
            let mut sum = 0.0;
            sum += x_1;
            sum += x_2;
            sum += x_1 * x_2 * 0.25;
            sum *= 0.02 + parameter;
            let scale: f32 = 2048.0 / ((1.0 + 1.0 + 0.25) * 1.02);
            interpolate_at(&LUT_BIPOLAR_FOLD, 2048, sum, scale, guarded)
        }
        ANALOG_RING => {
            let modulator = x_1;
            let carrier = x_2 * 2.0;
            let mut ring = diode(modulator + carrier) + diode(modulator - carrier);
            ring *= 4.0 + parameter * 24.0;
            soft_limit(ring)
        }
        DIGITAL_RING => {
            let ring = 4.0 * x_1 * x_2 * (1.0 + parameter * 8.0);
            ring / (1.0 + ring.abs())
        }
        XOR => {
            let x_1_short = clip16((x_1 * 32768.0) as i32) as i16;
            let x_2_short = clip16((x_2 * 32768.0) as i32) as i16;
            let modulation = (x_1_short ^ x_2_short) as f32 / 32768.0;
            let sum = (x_1 + x_2) * 0.7;
            sum + (modulation - sum) * parameter
        }
        COMPARATOR => {
            let (modulator, carrier) = (x_1, x_2);
            let x = parameter * 2.995;
            let mut integral = x as i32;
            let fractional = x - integral as f32;
            let direct = if modulator < carrier {
                modulator
            } else {
                carrier
            };
            let window = if modulator.abs() > carrier.abs() {
                modulator
            } else {
                carrier
            };
            let window_2 = if modulator.abs() > carrier.abs() {
                modulator.abs()
            } else {
                -carrier.abs()
            };
            let threshold = if carrier > 0.05 { carrier } else { modulator };
            let sequence = [direct, threshold, window, window_2];
            if !(0..=2).contains(&integral) {
                *guarded = guarded.saturating_add(1);
                integral = integral.clamp(0, 2);
            }
            let a = sequence[integral as usize];
            let b = sequence[integral as usize + 1];
            a + (b - a) * fractional
        }
        _ => x_1,
    }
}

/// Source `ProcessXmod<A, B>`; `carrier_out` is both input 2 and the output,
/// exactly as the source aliases its oversampled carrier/output buffers.
fn process_xmod<const A: u8, const B: u8>(
    mut balance: f32,
    balance_end: f32,
    mut parameter: f32,
    parameter_end: f32,
    modulator: &[f32],
    carrier_out: &mut [f32],
    guarded: &mut u32,
) {
    let size = carrier_out.len();
    let step = 1.0 / size as f32;
    let parameter_increment = (parameter_end - parameter) * step;
    let balance_increment = (balance_end - balance) * step;
    for (x_1, out) in modulator.iter().zip(carrier_out.iter_mut()) {
        let x_2 = *out;
        let a = xmod::<A>(*x_1, x_2, parameter, guarded);
        let b = xmod::<B>(*x_1, x_2, parameter, guarded);
        *out = a + (b - a) * balance;
        parameter += parameter_increment;
        balance += balance_increment;
    }
}

// ---------------------------------------------------------------------------
// Modulator

#[derive(Clone, Copy, Debug)]
pub(crate) struct Modulator {
    pub(crate) parameters: Parameters,
    previous_parameters: Parameters,
    pub(crate) easter_egg: bool,
    amplifier: [SaturatingAmplifier; 2],
    xmod_oscillator: Oscillator,
    vocoder_oscillator: Oscillator,
    quadrature_oscillator: QuadratureOscillator,
    src_up: [SrcUp<6, 48, 8>; 2],
    src_down: SrcDown<6, 48, 96>,
    vocoder: Vocoder,
    quadrature_transform: [QuadratureTransform; 2],
    internal_modulation: [f32; MAX_BLOCK],
    buffer: [[f32; MAX_BLOCK]; 3],
    src_buffer: [[f32; MAX_BLOCK * OVERSAMPLING]; 2],
    feedback_sample: f32,
    rng: u32,
    /// Count of source table reads that would have left their table.
    pub(crate) guarded_reads: u32,
}

impl Modulator {
    /// Equivalent to a zero-initialized static instance followed by Init.
    pub(crate) fn new(sample_rate: f32) -> Self {
        let mut modulator = Self {
            parameters: Parameters::default(),
            previous_parameters: Parameters::default(),
            easter_egg: false,
            amplifier: Default::default(),
            xmod_oscillator: Oscillator::default(),
            vocoder_oscillator: Oscillator::default(),
            quadrature_oscillator: QuadratureOscillator::default(),
            src_up: [
                SrcUp::new(&resources::SRC_UP_6_48),
                SrcUp::new(&resources::SRC_UP_6_48),
            ],
            src_down: SrcDown::new(&resources::SRC_DOWN_6_48),
            vocoder: Vocoder::new(),
            quadrature_transform: [
                QuadratureTransform::default(),
                QuadratureTransform::default(),
            ],
            internal_modulation: [0.0; MAX_BLOCK],
            buffer: [[0.0; MAX_BLOCK]; 3],
            src_buffer: [[0.0; MAX_BLOCK * OVERSAMPLING]; 2],
            feedback_sample: 0.0,
            rng: 0x21,
            guarded_reads: 0,
        };
        modulator.init(sample_rate);
        modulator
    }

    /// Return to the freshly constructed state in place (no large temporary).
    pub(crate) fn reinitialize(&mut self, sample_rate: f32) {
        self.parameters = Parameters::default();
        self.previous_parameters = Parameters::default();
        self.amplifier = Default::default();
        self.xmod_oscillator = Oscillator::default();
        self.vocoder_oscillator = Oscillator::default();
        self.quadrature_oscillator = QuadratureOscillator::default();
        self.vocoder.clear();
        self.quadrature_transform = Default::default();
        self.internal_modulation.fill(0.0);
        for buffer in &mut self.buffer {
            buffer.fill(0.0);
        }
        for buffer in &mut self.src_buffer {
            buffer.fill(0.0);
        }
        self.feedback_sample = 0.0;
        self.rng = 0x21;
        self.guarded_reads = 0;
        self.init(sample_rate);
    }

    fn init(&mut self, sample_rate: f32) {
        self.easter_egg = false;
        for i in 0..2 {
            self.amplifier[i].init();
            self.src_up[i].init();
            self.quadrature_transform[i].init(&LUT_AP_POLES);
        }
        self.src_down.init();
        self.xmod_oscillator.init(sample_rate);
        self.vocoder_oscillator.init(sample_rate);
        self.quadrature_oscillator.init(sample_rate);
        self.vocoder.init(sample_rate);
        self.previous_parameters.carrier_shape = 0;
        self.previous_parameters.channel_drive = [0.0; 2];
        self.previous_parameters.modulation_algorithm = 0.0;
        self.previous_parameters.modulation_parameter = 0.0;
        self.previous_parameters.note = 48.0;
        self.feedback_sample = 0.0;
    }

    /// Source `Modulator::Process` (bypass is a host policy, not kernel state).
    pub(crate) fn process(&mut self, input: &[[i16; 2]], output: &mut [[i16; 2]]) {
        let size = input.len();
        assert!(size > 0 && size <= BLOCK && size.is_multiple_of(12) && output.len() == size);
        if self.easter_egg {
            self.process_easter_egg(input, output);
            return;
        }
        let parameters = self.parameters;
        let guarded = &mut self.guarded_reads;

        // 0.0: use cross-modulation algorithms. 1.0: use vocoder.
        let vocoder_amount = constrain(
            (parameters.modulation_algorithm - 0.7) * 20.0 + 0.5,
            0.0,
            1.0,
        );

        let [carrier, modulator, aux_output] = &mut self.buffer;
        let carrier = &mut carrier[..size];
        let modulator = &mut modulator[..size];
        let aux_output = &mut aux_output[..size];
        if parameters.carrier_shape == 0 {
            aux_output.fill(0.0);
        }

        // Convert audio inputs to float and apply VCA/saturation.
        let first = if parameters.carrier_shape != 0 { 1 } else { 0 };
        for i in first..2 {
            let destination = if i == 0 {
                &mut *carrier
            } else {
                &mut *modulator
            };
            self.amplifier[i].process(
                parameters.channel_drive[i],
                1.0 - vocoder_amount,
                input,
                i,
                destination,
                aux_output,
            );
        }

        // If necessary, render carrier. Otherwise, the amplifiers summed both
        // signals into the aux output.
        if parameters.carrier_shape != 0 {
            let internal_modulation = &mut self.internal_modulation[..size];
            for (modulation, frame) in internal_modulation.iter_mut().zip(input) {
                *modulation = frame[0] as f32 / 32768.0;
            }
            // Xmod: sine, triangle, saw. Vocoder: saw, pulse, noise.
            let xmod_shape = (parameters.carrier_shape - 1) as usize;
            let vocoder_shape = (parameters.carrier_shape + 1) as usize;
            const XMOD_CARRIER_GAIN: f32 = 0.5;
            if vocoder_amount == 0.0 {
                self.xmod_oscillator.render(
                    xmod_shape,
                    parameters.note,
                    internal_modulation,
                    aux_output,
                    &mut self.rng,
                );
                for (c, a) in carrier.iter_mut().zip(aux_output.iter()) {
                    *c = *a * XMOD_CARRIER_GAIN;
                }
            } else if vocoder_amount >= 0.5 {
                let carrier_gain = self.vocoder_oscillator.render(
                    vocoder_shape,
                    parameters.note,
                    internal_modulation,
                    aux_output,
                    &mut self.rng,
                );
                for (c, a) in carrier.iter_mut().zip(aux_output.iter()) {
                    *c = *a * carrier_gain;
                }
            } else {
                let balance = vocoder_amount * 2.0;
                self.xmod_oscillator.render(
                    xmod_shape,
                    parameters.note,
                    internal_modulation,
                    carrier,
                    &mut self.rng,
                );
                let carrier_gain = self.vocoder_oscillator.render(
                    vocoder_shape,
                    parameters.note,
                    internal_modulation,
                    aux_output,
                    &mut self.rng,
                );
                for (c, aux) in carrier.iter_mut().zip(aux_output.iter_mut()) {
                    let mut a = *c;
                    let mut b = *aux;
                    *aux = a + (b - a) * balance;
                    a *= XMOD_CARRIER_GAIN;
                    b *= carrier_gain;
                    *c = a + (b - a) * balance;
                }
            }
        }

        if vocoder_amount < 0.5 {
            let oversampled = size * OVERSAMPLING;
            let [oversampled_carrier, oversampled_modulator] = &mut self.src_buffer;
            self.src_up[0].process(carrier, &mut oversampled_carrier[..oversampled]);
            self.src_up[1].process(modulator, &mut oversampled_modulator[..oversampled]);

            let algorithm = (parameters.modulation_algorithm * 8.0).min(5.999);
            let previous_algorithm =
                (self.previous_parameters.modulation_algorithm * 8.0).min(5.999);
            let algorithm_integral = algorithm as i32;
            let algorithm_fractional = algorithm - algorithm_integral as f32;
            let previous_algorithm_integral = previous_algorithm as i32;
            let mut previous_algorithm_fractional =
                previous_algorithm - previous_algorithm_integral as f32;
            if algorithm_integral != previous_algorithm_integral {
                previous_algorithm_fractional = algorithm_fractional;
            }
            let modulator_in = &oversampled_modulator[..oversampled];
            let output_buffer = &mut oversampled_carrier[..oversampled];
            let start = self.previous_parameters.skewed_modulation_parameter();
            let end = parameters.skewed_modulation_parameter();
            let (b0, b1) = (previous_algorithm_fractional, algorithm_fractional);
            match algorithm_integral {
                0 => process_xmod::<XFADE, FOLD>(
                    b0,
                    b1,
                    start,
                    end,
                    modulator_in,
                    output_buffer,
                    guarded,
                ),
                1 => process_xmod::<FOLD, ANALOG_RING>(
                    b0,
                    b1,
                    start,
                    end,
                    modulator_in,
                    output_buffer,
                    guarded,
                ),
                2 => process_xmod::<ANALOG_RING, DIGITAL_RING>(
                    b0,
                    b1,
                    start,
                    end,
                    modulator_in,
                    output_buffer,
                    guarded,
                ),
                3 => process_xmod::<DIGITAL_RING, XOR>(
                    b0,
                    b1,
                    start,
                    end,
                    modulator_in,
                    output_buffer,
                    guarded,
                ),
                4 => process_xmod::<XOR, COMPARATOR>(
                    b0,
                    b1,
                    start,
                    end,
                    modulator_in,
                    output_buffer,
                    guarded,
                ),
                _ => process_xmod::<COMPARATOR, NOP>(
                    b0,
                    b1,
                    start,
                    end,
                    modulator_in,
                    output_buffer,
                    guarded,
                ),
            }
            // The main output shares buffer 0 with the carrier.
            self.src_down.process(output_buffer, carrier);
        } else {
            let release_time = constrain(4.0 * (parameters.modulation_algorithm - 0.75), 0.0, 1.0);
            self.vocoder
                .set_release_time(release_time * (2.0 - release_time));
            self.vocoder
                .set_formant_shift(parameters.modulation_parameter);
            // Source passes the carrier and main output as the same buffer.
            self.vocoder.process(modulator, carrier, guarded);
        }
        let main_output = carrier;

        // Cross-fade to raw modulator for the transition between
        // cross-modulation algorithms and vocoding algorithms.
        let transition_gain = 2.0
            * if vocoder_amount < 0.5 {
                vocoder_amount
            } else {
                1.0 - vocoder_amount
            };
        if transition_gain != 0.0 {
            for (main, m) in main_output.iter_mut().zip(modulator.iter()) {
                *main += transition_gain * (*m - *main);
            }
        }

        // Convert back to integer and clip.
        for ((frame, main), aux) in output
            .iter_mut()
            .zip(main_output.iter())
            .zip(aux_output.iter())
        {
            frame[0] = clip16((*main * 32768.0) as i32) as i16;
            frame[1] = clip16((*aux * 16384.0) as i32) as i16;
        }
        self.previous_parameters = parameters;
    }

    /// Source `Modulator::ProcessEasterEgg`: the frequency shifter.
    fn process_easter_egg(&mut self, input: &[[i16; 2]], output: &mut [[i16; 2]]) {
        let size = input.len();
        let parameters = self.parameters;
        let guarded = &mut self.guarded_reads;
        let (carrier_i, carrier_q) = self.src_buffer[0][..2 * size].split_at_mut(size);

        // Generate the I/Q components.
        if parameters.carrier_shape != 0 {
            let d = parameters.frequency_shift_pot - 0.5;
            let mut linear_modulation_amount = 1.0 - 14.0 * d * d;
            if linear_modulation_amount < 0.0 {
                linear_modulation_amount = 0.0;
            }
            let mut frequency = parameters.frequency_shift_pot;
            frequency += linear_modulation_amount * parameters.frequency_shift_cv;
            let direction = if frequency >= 0.5 { 1.0 } else { -1.0 };
            frequency = 2.0 * (frequency - 0.5).abs();
            frequency = if frequency <= 0.4 {
                frequency * frequency * frequency * 62.5
            } else {
                4.0 * semitones_to_ratio(180.0 * (frequency - 0.4), guarded)
            };
            frequency *= semitones_to_ratio(
                parameters.frequency_shift_cv * 60.0 * (1.0 - linear_modulation_amount) * direction,
                guarded,
            );
            frequency *= direction;
            let shape = (parameters.carrier_shape - 1) as f32 * 0.5;
            self.quadrature_oscillator
                .render(shape, frequency, carrier_i, carrier_q, guarded);
        } else {
            let carrier = &mut self.buffer[0][..size];
            for (c, frame) in carrier.iter_mut().zip(input) {
                *c = frame[0] as f32 / 32768.0;
            }
            self.quadrature_transform[0].process_block(carrier, carrier_i, carrier_q);
            let mut phase_shift = Interpolator::new(
                self.previous_parameters.phase_shift,
                parameters.phase_shift,
                size,
            );
            for (i_out, q_out) in carrier_i.iter_mut().zip(carrier_q.iter_mut()) {
                let x_i = *i_out;
                let x_q = *q_out;
                let angle = phase_shift.next();
                let r_sin = interpolate(&LUT_SIN, angle, 1024.0, guarded);
                let r_cos = interpolate(&LUT_SIN[256..], angle, 1024.0, guarded);
                *i_out = r_sin * x_i + r_cos * x_q;
                *q_out = r_sin * x_q - r_cos * x_i;
            }
            self.previous_parameters.phase_shift = phase_shift.value();
        }

        // Setup parameter interpolation.
        let mut mix = Interpolator::new(
            self.previous_parameters.modulation_parameter,
            parameters.modulation_parameter,
            size,
        );
        let mut feedback_amount = Interpolator::new(
            self.previous_parameters.channel_drive[0],
            parameters.channel_drive[0],
            size,
        );
        let mut dry_wet = Interpolator::new(
            self.previous_parameters.channel_drive[1],
            parameters.channel_drive[1],
            size,
        );

        let mut feedback_sample = self.feedback_sample;
        for i in 0..size {
            let timbre = mix.next();

            // Start from the signal from input 2, with non-linear gain.
            let mut input_sample = input[i][1] as f32 / 32768.0;
            if parameters.carrier_shape != 0 {
                input_sample += input[i][0] as f32 / 32768.0;
            }
            let mut modulator = input_sample;

            // Apply feedback if necessary, and soft limit.
            let mut amount = feedback_amount.next();
            amount *= 2.0 - amount;
            amount *= 2.0 - amount;

            // mic.w feedback amount tweak.
            let max_fb = 1.0 + 2.0 * (timbre - 0.5) * (timbre - 0.5);
            modulator +=
                amount * (soft_clip(modulator + max_fb * feedback_sample * amount) - modulator);

            let (modulator_i, modulator_q) = self.quadrature_transform[1].process(modulator);

            // Modulate!
            let a = carrier_i[i] * modulator_i;
            let b = carrier_q[i] * modulator_q;
            let up = a - b;
            let down = a + b;
            let fade_in = interpolate(&LUT_XFADE_IN, timbre, 256.0, guarded);
            let fade_out = interpolate(&LUT_XFADE_OUT, timbre, 256.0, guarded);
            let mut main = up * fade_in + down * fade_out;
            let mut aux = down * fade_in + up * fade_out;

            // Simple LP to prevent feedback of high-frequencies.
            feedback_sample += 0.2 * (main - feedback_sample);

            let wet_dry = 1.0 - dry_wet.next();
            main += wet_dry * (input_sample - main);
            aux += wet_dry * (input_sample - aux);

            output[i][0] = clip16((main * 32768.0) as i32) as i16;
            output[i][1] = clip16((aux * 32768.0) as i32) as i16;
        }
        self.feedback_sample = feedback_sample;
        self.previous_parameters = parameters;
        // Source interpolator destructors run after that assignment, in
        // reverse construction order, and write back accumulated values.
        self.previous_parameters.channel_drive[1] = dry_wet.value();
        self.previous_parameters.channel_drive[0] = feedback_amount.value();
        self.previous_parameters.modulation_parameter = mix.value();
    }
}
