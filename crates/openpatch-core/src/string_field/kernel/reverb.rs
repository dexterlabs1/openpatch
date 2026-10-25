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
// Rust adaptation (OpenPatch String Field) of rings/dsp/fx/fx_engine.h and
// rings/dsp/fx/reverb.h at eurorack f4cfa7697095a28d292bff758beb51deb49b5a86:
// the Griesinger-style reverb of the string-and-reverb model, over the
// firmware's 32768-word 16-bit delay memory.

use super::stmlib::{clip16, trunc_i32, CosineOscillator};

pub(super) const REVERB_BUFFER_SIZE: usize = 32768;
const MASK: i32 = REVERB_BUFFER_SIZE as i32 - 1;

/// `E::Reserve<...>` lengths in declaration order: ap1..ap4, dap1a, dap1b,
/// del1, dap2a, dap2b, del2.
const LENGTHS: [i32; 10] = [150, 214, 319, 527, 2182, 2690, 4501, 2525, 2197, 6312];

/// `DelayLine<Memory, i>::base`: each line starts one word after the end of
/// the previous one.
const fn bases() -> [i32; 10] {
    let mut bases = [0; 10];
    let mut i = 1;
    while i < 10 {
        bases[i] = bases[i - 1] + LENGTHS[i - 1] + 1;
        i += 1;
    }
    bases
}
const BASES: [i32; 10] = bases();

const AP1: usize = 0;
const AP2: usize = 1;
const AP3: usize = 2;
const AP4: usize = 3;
const DAP1A: usize = 4;
const DAP1B: usize = 5;
const DEL1: usize = 6;
const DAP2A: usize = 7;
const DAP2B: usize = 8;
const DEL2: usize = 9;

/// `DataType<FORMAT_16_BIT>::Decompress`.
#[inline]
fn decompress(value: u16) -> f32 {
    value as i16 as f32 / 32768.0
}

/// `DataType<FORMAT_16_BIT>::Compress`; out-of-range conversions saturate on
/// AArch64, Cortex-M and in Rust.
#[inline]
fn compress(value: f32) -> u16 {
    clip16(trunc_i32(value * 32768.0)) as i16 as u16
}

/// `FxEngine::Context`.
struct Context<'a> {
    accumulator: f32,
    previous_read: f32,
    lfo_value: [f32; 2],
    buffer: &'a mut [u16],
    write_ptr: i32,
}

impl Context<'_> {
    #[inline]
    fn load(&mut self, value: f32) {
        self.accumulator = value;
    }

    #[inline]
    fn read_value(&mut self, value: f32, scale: f32) {
        self.accumulator += value * scale;
    }

    #[inline]
    fn write_value(&mut self, scale: f32) -> f32 {
        let value = self.accumulator;
        self.accumulator *= scale;
        value
    }

    #[inline]
    fn index(&self, offset: i32) -> usize {
        (self.write_ptr.wrapping_add(offset) & MASK) as usize
    }

    #[inline]
    fn write(&mut self, line: usize, scale: f32) {
        let w = compress(self.accumulator);
        let index = self.index(BASES[line]);
        self.buffer[index] = w;
        self.accumulator *= scale;
    }

    #[inline]
    fn write_all_pass(&mut self, line: usize, scale: f32) {
        self.write(line, scale);
        self.accumulator += self.previous_read;
    }

    /// `Read(d TAIL, scale)`: the oldest sample of the line.
    #[inline]
    fn read_tail(&mut self, line: usize, scale: f32) {
        let r = self.buffer[self.index(BASES[line] + LENGTHS[line] - 1)];
        let r_f = decompress(r);
        self.previous_read = r_f;
        self.accumulator += r_f * scale;
    }

    #[inline]
    fn lp(&mut self, state: &mut f32, coefficient: f32) {
        *state += coefficient * (self.accumulator - *state);
        self.accumulator = *state;
    }

    #[inline]
    fn interpolate(&mut self, line: usize, offset: f32, lfo: usize, amplitude: f32, scale: f32) {
        let offset = offset + amplitude * self.lfo_value[lfo];
        let integral = trunc_i32(offset);
        let fractional = offset - integral as f32;
        let a = decompress(self.buffer[self.index(integral + BASES[line])]);
        let b = decompress(self.buffer[self.index(integral + BASES[line] + 1)]);
        let x = a + (b - a) * fractional;
        self.previous_read = x;
        self.accumulator += x * scale;
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Reverb {
    write_ptr: i32,
    lfo: [CosineOscillator; 2],
    amount: f32,
    input_gain: f32,
    reverb_time: f32,
    diffusion: f32,
    lp: f32,
    lp_decay_1: f32,
    lp_decay_2: f32,
}

impl Reverb {
    pub(super) const ZERO: Self = Self {
        write_ptr: 0,
        lfo: [CosineOscillator::ZERO; 2],
        amount: 0.0,
        input_gain: 0.0,
        reverb_time: 0.0,
        diffusion: 0.0,
        lp: 0.0,
        lp_decay_1: 0.0,
        lp_decay_2: 0.0,
    };

    pub(super) fn init(&mut self, buffer: &mut [u16]) {
        buffer.fill(0);
        self.write_ptr = 0;
        self.lfo[0].init_approximate(0.5 / 48000.0 * 32.0);
        self.lfo[1].init_approximate(0.3 / 48000.0 * 32.0);
        self.lp = 0.7;
        self.diffusion = 0.625;
    }

    pub(super) fn set_amount(&mut self, amount: f32) {
        self.amount = amount;
    }

    pub(super) fn set_input_gain(&mut self, input_gain: f32) {
        self.input_gain = input_gain;
    }

    pub(super) fn set_time(&mut self, reverb_time: f32) {
        self.reverb_time = reverb_time;
    }

    pub(super) fn set_diffusion(&mut self, diffusion: f32) {
        self.diffusion = diffusion;
    }

    pub(super) fn set_lp(&mut self, lp: f32) {
        self.lp = lp;
    }

    /// `FxEngine::Start`.
    fn start<'a>(&mut self, buffer: &'a mut [u16]) -> Context<'a> {
        self.write_ptr -= 1;
        if self.write_ptr < 0 {
            self.write_ptr += REVERB_BUFFER_SIZE as i32;
        }
        let lfo_value = if self.write_ptr & 31 == 0 {
            [self.lfo[0].next(), self.lfo[1].next()]
        } else {
            [self.lfo[0].value(), self.lfo[1].value()]
        };
        Context {
            accumulator: 0.0,
            previous_read: 0.0,
            lfo_value,
            buffer,
            write_ptr: self.write_ptr,
        }
    }

    pub(super) fn process(&mut self, buffer: &mut [u16], left: &mut [f32], right: &mut [f32]) {
        let kap = self.diffusion;
        let klp = self.lp;
        let krt = self.reverb_time;
        let amount = self.amount;
        let gain = self.input_gain;
        let mut lp_1 = self.lp_decay_1;
        let mut lp_2 = self.lp_decay_2;

        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let mut c = self.start(buffer);

            c.read_value(*l + *r, gain);
            c.read_tail(AP1, kap);
            c.write_all_pass(AP1, -kap);
            c.read_tail(AP2, kap);
            c.write_all_pass(AP2, -kap);
            c.read_tail(AP3, kap);
            c.write_all_pass(AP3, -kap);
            c.read_tail(AP4, kap);
            c.write_all_pass(AP4, -kap);
            let apout = c.write_value(1.0);

            c.load(apout);
            c.interpolate(DEL2, 6261.0, 1, 50.0, krt);
            c.lp(&mut lp_1, klp);
            c.read_tail(DAP1A, -kap);
            c.write_all_pass(DAP1A, kap);
            c.read_tail(DAP1B, kap);
            c.write_all_pass(DAP1B, -kap);
            c.write(DEL1, 2.0);
            let wet = c.write_value(0.0);
            *l += (wet - *l) * amount;

            c.load(apout);
            c.interpolate(DEL1, 4460.0, 0, 40.0, krt);
            c.lp(&mut lp_2, klp);
            c.read_tail(DAP2A, kap);
            c.write_all_pass(DAP2A, -kap);
            c.read_tail(DAP2B, -kap);
            c.write_all_pass(DAP2B, kap);
            c.write(DEL2, 2.0);
            let wet = c.write_value(0.0);
            *r += (wet - *r) * amount;
        }

        self.lp_decay_1 = lp_1;
        self.lp_decay_2 = lp_2;
    }
}
