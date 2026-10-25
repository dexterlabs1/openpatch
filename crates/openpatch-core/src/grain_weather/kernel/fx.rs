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
// Rust adaptation (OpenPatch Grain Weather) of clouds/dsp/fx/fx_engine.h,
// diffuser.h, reverb.h, pitch_shifter.h and clouds/dsp/sample_rate_converter.h
// at eurorack f4cfa7697095a28d292bff758beb51deb49b5a86. Delay memory lives in
// the preallocated memory image at the addresses the source allocator picks.

use super::memory::Memory;
use super::resources::SRC_FILTER_1X_2_45;
use super::stmlib::{clip16, trunc_i32, CosineOscillator};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Format {
    #[default]
    Bits12,
    Bits16,
    Float,
}

/// Delay-line bases from the source's `Reserve<...>` chains: each line starts
/// one cell after the previous line's end.
const fn bases<const N: usize>(lengths: [usize; N]) -> [usize; N] {
    let mut result = [0; N];
    let mut i = 1;
    while i < N {
        result[i] = result[i - 1] + lengths[i - 1] + 1;
        i += 1;
    }
    result
}

#[derive(Clone, Copy, Debug)]
struct Line {
    base: i32,
    length: i32,
}

const fn lines<const N: usize>(lengths: [usize; N]) -> [Line; N] {
    let b = bases(lengths);
    let mut result = [Line { base: 0, length: 0 }; N];
    let mut i = 0;
    while i < N {
        result[i] = Line {
            base: b[i] as i32,
            length: lengths[i] as i32,
        };
        i += 1;
    }
    result
}

#[derive(Clone, Copy, Debug, Default)]
struct FxEngine {
    base: usize,
    mask: i32,
    format: Format,
    write_ptr: i32,
    lfo: [CosineOscillator; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct Context {
    accumulator: f32,
    previous_read: f32,
    lfo_value: [f32; 2],
    write_ptr: i32,
}

impl FxEngine {
    fn new(format: Format, size: i32) -> Self {
        Self {
            format,
            mask: size - 1,
            ..Self::default()
        }
    }

    fn init(&mut self, memory: &mut Memory, base: usize) {
        self.base = base;
        self.clear(memory);
    }

    fn clear(&mut self, memory: &mut Memory) {
        let size = (self.mask + 1) as usize;
        match self.format {
            Format::Float => memory.fill_u32(self.base, size, 0),
            _ => memory.fill_u16(self.base, size, 0),
        }
        self.write_ptr = 0;
    }

    fn set_lfo_frequency(&mut self, index: usize, frequency: f32) {
        self.lfo[index].init_approximate(frequency * 32.0);
    }

    #[inline]
    fn start(&mut self) -> Context {
        self.write_ptr -= 1;
        if self.write_ptr < 0 {
            self.write_ptr += self.mask + 1;
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
            write_ptr: self.write_ptr,
        }
    }

    #[inline]
    fn load(&self, memory: &Memory, index: i32) -> f32 {
        let index = (index & self.mask) as usize;
        match self.format {
            Format::Float => memory.f32(self.base + 4 * index),
            Format::Bits12 => memory.i16(self.base + 2 * index) as f32 / 4096.0,
            Format::Bits16 => memory.i16(self.base + 2 * index) as f32 / 32768.0,
        }
    }

    #[inline]
    fn store(&self, memory: &mut Memory, index: i32, value: f32) {
        let index = (index & self.mask) as usize;
        match self.format {
            Format::Float => memory.set_f32(self.base + 4 * index, value),
            Format::Bits12 => memory.set_i16(
                self.base + 2 * index,
                clip16(trunc_i32(value * 4096.0)) as i16,
            ),
            Format::Bits16 => memory.set_i16(
                self.base + 2 * index,
                clip16(trunc_i32(value * 32768.0)) as i16,
            ),
        }
    }

    #[inline]
    fn cell(c: &Context, line: Line, offset: i32) -> i32 {
        if offset == -1 {
            c.write_ptr + line.base + line.length - 1
        } else {
            c.write_ptr + line.base + offset
        }
    }

    #[inline]
    fn write(&self, memory: &mut Memory, c: &mut Context, line: Line, offset: i32, scale: f32) {
        self.store(memory, Self::cell(c, line, offset), c.accumulator);
        c.accumulator *= scale;
    }

    #[inline]
    fn write_all_pass(
        &self,
        memory: &mut Memory,
        c: &mut Context,
        line: Line,
        offset: i32,
        scale: f32,
    ) {
        self.write(memory, c, line, offset, scale);
        c.accumulator += c.previous_read;
    }

    #[inline]
    fn read(&self, memory: &Memory, c: &mut Context, line: Line, offset: i32, scale: f32) {
        let r = self.load(memory, Self::cell(c, line, offset));
        c.previous_read = r;
        c.accumulator += r * scale;
    }

    #[inline]
    fn interpolate(&self, memory: &Memory, c: &mut Context, line: Line, offset: f32, scale: f32) {
        let integral = trunc_i32(offset);
        let fractional = offset - integral as f32;
        let a = self.load(memory, c.write_ptr + integral + line.base);
        let b = self.load(memory, c.write_ptr + integral + line.base + 1);
        let x = a + (b - a) * fractional;
        c.previous_read = x;
        c.accumulator += x * scale;
    }

    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn interpolate_lfo(
        &self,
        memory: &Memory,
        c: &mut Context,
        line: Line,
        offset: f32,
        lfo: usize,
        amplitude: f32,
        scale: f32,
    ) {
        let offset = offset + amplitude * c.lfo_value[lfo];
        self.interpolate(memory, c, line, offset, scale);
    }
}

#[inline]
fn lp(c: &mut Context, state: &mut f32, coefficient: f32) {
    *state += coefficient * (c.accumulator - *state);
    c.accumulator = *state;
}

const DIFFUSER: [Line; 8] = lines([126, 180, 269, 444, 151, 205, 245, 405]);

/// `clouds::Diffuser`: four all-passes per channel, 32-bit float memory.
#[derive(Clone, Copy, Debug)]
pub(super) struct Diffuser {
    engine: FxEngine,
    amount: f32,
}

impl Diffuser {
    pub(super) fn new() -> Self {
        Self {
            engine: FxEngine::new(Format::Float, 2048),
            amount: 0.0,
        }
    }

    pub(super) fn init(&mut self, memory: &mut Memory, base: usize) {
        self.engine.init(memory, base);
    }

    pub(super) fn set_amount(&mut self, amount: f32) {
        self.amount = amount;
    }

    pub(super) fn process(&mut self, memory: &mut Memory, in_out: &mut [[f32; 2]]) {
        const KAP: f32 = 0.625;
        for frame in in_out.iter_mut() {
            let mut c = self.engine.start();
            let e = self.engine;
            for channel in 0..2 {
                let lines = &DIFFUSER[channel * 4..channel * 4 + 4];
                c.accumulator += frame[channel];
                for &line in lines {
                    e.read(memory, &mut c, line, -1, KAP);
                    e.write_all_pass(memory, &mut c, line, 0, -KAP);
                }
                let wet = c.accumulator;
                c.accumulator *= 0.0;
                frame[channel] += self.amount * (wet - frame[channel]);
            }
        }
    }
}

const REVERB: [Line; 10] = lines([113, 162, 241, 399, 1653, 2038, 3411, 1913, 1663, 4782]);

/// `clouds::Reverb`: Griesinger topology with 12-bit delay memory.
#[derive(Clone, Copy, Debug)]
pub(super) struct Reverb {
    engine: FxEngine,
    amount: f32,
    input_gain: f32,
    reverb_time: f32,
    diffusion: f32,
    lp: f32,
    lp_decay_1: f32,
    lp_decay_2: f32,
}

impl Reverb {
    pub(super) fn new() -> Self {
        Self {
            engine: FxEngine::new(Format::Bits12, 16384),
            amount: 0.0,
            input_gain: 0.0,
            reverb_time: 0.0,
            diffusion: 0.0,
            lp: 0.0,
            lp_decay_1: 0.0,
            lp_decay_2: 0.0,
        }
    }

    /// The source does not reset the loop low-pass states here.
    pub(super) fn init(&mut self, memory: &mut Memory, base: usize) {
        self.engine.init(memory, base);
        self.engine.set_lfo_frequency(0, 0.5 / 32000.0);
        self.engine.set_lfo_frequency(1, 0.3 / 32000.0);
        self.lp = 0.7;
        self.diffusion = 0.625;
    }

    pub(super) fn set_amount(&mut self, amount: f32) {
        self.amount = amount;
    }
    pub(super) fn set_input_gain(&mut self, input_gain: f32) {
        self.input_gain = input_gain;
    }
    pub(super) fn set_time(&mut self, time: f32) {
        self.reverb_time = time;
    }
    pub(super) fn set_diffusion(&mut self, diffusion: f32) {
        self.diffusion = diffusion;
    }
    pub(super) fn set_lp(&mut self, lp: f32) {
        self.lp = lp;
    }

    pub(super) fn process(&mut self, memory: &mut Memory, in_out: &mut [[f32; 2]]) {
        let [ap1, ap2, ap3, ap4, dap1a, dap1b, del1, dap2a, dap2b, del2] = REVERB;
        let kap = self.diffusion;
        let klp = self.lp;
        let krt = self.reverb_time;
        let amount = self.amount;
        let gain = self.input_gain;
        let mut lp_1 = self.lp_decay_1;
        let mut lp_2 = self.lp_decay_2;
        for frame in in_out.iter_mut() {
            let mut c = self.engine.start();
            let e = self.engine;
            // Smear AP1 inside the loop.
            e.interpolate_lfo(memory, &mut c, ap1, 10.0, 0, 60.0, 1.0);
            e.write(memory, &mut c, ap1, 100, 0.0);

            c.accumulator += (frame[0] + frame[1]) * gain;

            // Diffuse through 4 allpasses.
            for line in [ap1, ap2, ap3, ap4] {
                e.read(memory, &mut c, line, -1, kap);
                e.write_all_pass(memory, &mut c, line, 0, -kap);
            }
            // Main reverb loop (`Load(apout)` restores the same value).
            let apout = c.accumulator;
            e.interpolate_lfo(memory, &mut c, del2, 4680.0, 1, 100.0, krt);
            lp(&mut c, &mut lp_1, klp);
            e.read(memory, &mut c, dap1a, -1, -kap);
            e.write_all_pass(memory, &mut c, dap1a, 0, kap);
            e.read(memory, &mut c, dap1b, -1, kap);
            e.write_all_pass(memory, &mut c, dap1b, 0, -kap);
            e.write(memory, &mut c, del1, 0, 2.0);
            let wet = c.accumulator;
            c.accumulator *= 0.0;
            frame[0] += (wet - frame[0]) * amount;

            c.accumulator = apout;
            e.read(memory, &mut c, del1, -1, krt);
            lp(&mut c, &mut lp_2, klp);
            e.read(memory, &mut c, dap2a, -1, kap);
            e.write_all_pass(memory, &mut c, dap2a, 0, -kap);
            e.read(memory, &mut c, dap2b, -1, -kap);
            e.write_all_pass(memory, &mut c, dap2b, 0, kap);
            e.write(memory, &mut c, del2, 0, 2.0);
            let wet = c.accumulator;
            frame[1] += (wet - frame[1]) * amount;
        }
        self.lp_decay_1 = lp_1;
        self.lp_decay_2 = lp_2;
    }
}

const PITCH_SHIFTER: [Line; 2] = lines([2047, 2047]);

/// `clouds::PitchShifter`: two crossfaded delay taps, 16-bit memory.
#[derive(Clone, Copy, Debug)]
pub(super) struct PitchShifter {
    engine: FxEngine,
    phase: f32,
    ratio: f32,
    size: f32,
}

impl PitchShifter {
    pub(super) fn new() -> Self {
        Self {
            engine: FxEngine::new(Format::Bits16, 4096),
            phase: 0.0,
            ratio: 0.0,
            size: 0.0,
        }
    }

    pub(super) fn init(&mut self, memory: &mut Memory, base: usize) {
        self.engine.init(memory, base);
        self.phase = 0.0;
        self.size = 2047.0;
    }

    pub(super) fn clear(&mut self, memory: &mut Memory) {
        self.engine.clear(memory);
    }

    pub(super) fn set_ratio(&mut self, ratio: f32) {
        self.ratio = ratio;
    }

    pub(super) fn set_size(&mut self, size: f32) {
        let target_size = 128.0 + (2047.0 - 128.0) * size * size * size;
        self.size += 0.05 * (target_size - self.size);
    }

    pub(super) fn process(&mut self, memory: &mut Memory, in_out: &mut [[f32; 2]]) {
        for frame in in_out.iter_mut() {
            let mut c = self.engine.start();
            let e = self.engine;
            self.phase += (1.0 - self.ratio) / self.size;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            if self.phase <= 0.0 {
                self.phase += 1.0;
            }
            let tri = 2.0
                * (if self.phase >= 0.5 {
                    1.0 - self.phase
                } else {
                    self.phase
                });
            let phase = self.phase * self.size;
            let mut half = phase + self.size * 0.5;
            if half >= self.size {
                half -= self.size;
            }
            for (channel, line) in PITCH_SHIFTER.into_iter().enumerate() {
                c.accumulator += frame[channel] * 1.0;
                e.write(memory, &mut c, line, 0, 0.0);
                e.interpolate(memory, &mut c, line, phase, tri);
                e.interpolate(memory, &mut c, line, half, 1.0 - tri);
                frame[channel] = c.accumulator;
                c.accumulator *= 0.0;
            }
        }
    }
}

const SRC_TAPS: usize = 45;

/// `SampleRateConverter<+-2, 45, src_filter_1x_2_45>`.
#[derive(Clone, Copy, Debug)]
pub(super) struct SampleRateConverter {
    ratio: i32,
    history: [[f32; 2]; SRC_TAPS * 2],
    history_ptr: i32,
}

impl SampleRateConverter {
    pub(super) fn new(ratio: i32) -> Self {
        let mut converter = Self {
            ratio,
            history: [[0.0; 2]; SRC_TAPS * 2],
            history_ptr: 0,
        };
        converter.init();
        converter
    }

    pub(super) fn init(&mut self) {
        self.history = [[0.0; 2]; SRC_TAPS * 2];
        self.history_ptr = SRC_TAPS as i32 - 1;
    }

    pub(super) fn process(&mut self, input: &[[f32; 2]], output: &mut [[f32; 2]]) {
        let filter_size = SRC_TAPS as i32;
        let scale = if self.ratio < 0 {
            1.0
        } else {
            self.ratio as f32
        };
        let consumed = if self.ratio < 0 { -self.ratio } else { 1 } as usize;
        let produced = if self.ratio > 0 { self.ratio } else { 1 } as usize;
        let mut history_ptr = self.history_ptr;
        let mut out = output.iter_mut();
        for chunk in input.chunks(consumed) {
            for &frame in chunk {
                self.history[(history_ptr + filter_size) as usize] = frame;
                self.history[history_ptr as usize] = frame;
                history_ptr -= 1;
                if history_ptr < 0 {
                    history_ptr += filter_size;
                }
            }
            for i in 0..produced {
                let mut y_l = 0.0f32;
                let mut y_r = 0.0f32;
                let mut x = (history_ptr + 1) as usize;
                let mut j = i;
                while j < SRC_TAPS {
                    let h = SRC_FILTER_1X_2_45[j];
                    y_l += self.history[x][0] * h;
                    y_r += self.history[x][1] * h;
                    x += 1;
                    j += produced;
                }
                if let Some(frame) = out.next() {
                    *frame = [y_l * scale, y_r * scale];
                }
            }
        }
        self.history_ptr = history_ptr;
    }
}
