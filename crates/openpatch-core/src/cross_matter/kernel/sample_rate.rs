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
// Rust adaptation of warps/dsp/sample_rate_converter.h (polyphase FIR up/down
// converters with mirrored half-length coefficient tables) and
// warps/dsp/quadrature_transform.h. The template Accumulator sums
// x[0]*h[0] + (x[1]*h[1] + (... + (x[N-1]*h[N-1] + 0))); the loops below keep
// that right-to-left association exactly (addition order, not operand order,
// determines the binary32 result).

/// Mirrored coefficient read: Accumulator `h_index` with `mirror = FILTER`.
#[inline(always)]
fn mirrored(h: &[f32], filter_size: usize, index: usize) -> f32 {
    if index >= filter_size / 2 {
        h[filter_size - 1 - index]
    } else {
        h[index]
    }
}

/// SampleRateConverter<SRC_UP, RATIO, FILTER>; N = FILTER / RATIO.
#[derive(Clone, Copy, Debug)]
pub(super) struct SrcUp<const RATIO: usize, const FILTER: usize, const N: usize> {
    x: [f32; N],
    h: &'static [f32],
}

impl<const RATIO: usize, const FILTER: usize, const N: usize> SrcUp<RATIO, FILTER, N> {
    pub(super) fn new(h: &'static [f32]) -> Self {
        assert!(N * RATIO == FILTER && h.len() == FILTER / 2);
        Self { x: [0.0; N], h }
    }

    pub(super) fn init(&mut self) {
        self.x = [0.0; N];
    }

    pub(super) const fn delay() -> usize {
        FILTER / RATIO / 2
    }

    pub(super) fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let mut out = 0;
        for &sample in input {
            // FilterState::Push: x[0] is the newest sample.
            for i in (1..N).rev() {
                self.x[i] = self.x[i - 1];
            }
            self.x[0] = sample;
            for offset in 0..RATIO {
                let mut accumulator = 0.0;
                for i in (0..N).rev() {
                    accumulator += self.x[i] * mirrored(self.h, FILTER, offset + i * RATIO);
                }
                output[out] = accumulator;
                out += 1;
            }
        }
    }
}

/// SampleRateConverter<SRC_DOWN, RATIO, FILTER>, circular-history branch;
/// TWO_N = 2 * FILTER. The source switches to a linear branch only when a
/// block holds at least 8 * FILTER input samples, which fixed blocks of at
/// most 60 frames never reach.
#[derive(Clone, Copy, Debug)]
pub(super) struct SrcDown<const RATIO: usize, const FILTER: usize, const TWO_N: usize> {
    x: [f32; TWO_N],
    pointer: usize,
    h: &'static [f32],
}

impl<const RATIO: usize, const FILTER: usize, const TWO_N: usize> SrcDown<RATIO, FILTER, TWO_N> {
    pub(super) fn new(h: &'static [f32]) -> Self {
        assert!(TWO_N == 2 * FILTER && h.len() == FILTER / 2);
        Self {
            x: [0.0; TWO_N],
            pointer: FILTER - 1,
            h,
        }
    }

    pub(super) fn init(&mut self) {
        self.x = [0.0; TWO_N];
        self.pointer = FILTER - 1;
    }

    pub(super) const fn delay() -> usize {
        FILTER / 2
    }

    pub(super) fn process(&mut self, input: &[f32], output: &mut [f32]) {
        if !input.len().is_multiple_of(RATIO) {
            return;
        }
        debug_assert!(input.len() < 8 * FILTER);
        for (frames, out) in input.as_chunks::<RATIO>().0.iter().zip(output.iter_mut()) {
            for &sample in frames {
                self.x[self.pointer] = sample;
                self.x[self.pointer + FILTER] = sample;
                self.pointer = if self.pointer == 0 {
                    FILTER - 1
                } else {
                    self.pointer - 1
                };
            }
            let history = &self.x[self.pointer + 1..self.pointer + 1 + FILTER];
            let mut accumulator = 0.0;
            for i in (0..FILTER).rev() {
                accumulator += history[i] * mirrored(self.h, FILTER, i);
            }
            *out = accumulator;
        }
    }
}

const NUM_POLES: usize = 17;

#[derive(Clone, Copy, Debug, Default)]
struct AllPassFilter {
    x: f32,
    y: f32,
    coefficient: f32,
}

impl AllPassFilter {
    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let y = self.coefficient * (input - self.y) + self.x;
        self.x = input;
        self.y = y;
        y
    }
}

/// QuadratureTransform with the 17 source all-pass poles.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct QuadratureTransform {
    filter: [AllPassFilter; NUM_POLES],
}

impl QuadratureTransform {
    pub(super) fn init(&mut self, poles: &[f32; NUM_POLES]) {
        for (filter, pole) in self.filter.iter_mut().zip(poles) {
            *filter = AllPassFilter {
                x: 0.0,
                y: 0.0,
                coefficient: -pole,
            };
        }
    }

    #[inline]
    pub(super) fn process(&mut self, input: f32) -> (f32, f32) {
        let mut i_out = 0.0;
        let mut q_out = 0.0;
        for (i, filter) in self.filter.iter_mut().enumerate() {
            let destination = if i & 1 != 0 { &mut q_out } else { &mut i_out };
            let source = if i <= 1 { input } else { *destination };
            *destination = filter.process(source);
        }
        (i_out, q_out)
    }

    pub(super) fn process_block(&mut self, input: &[f32], i_out: &mut [f32], q_out: &mut [f32]) {
        for (i, filter) in self.filter.iter_mut().enumerate() {
            let destination: &mut [f32] = if i & 1 != 0 { &mut *q_out } else { &mut *i_out };
            for (j, out) in destination.iter_mut().enumerate() {
                let source = if i <= 1 { input[j] } else { *out };
                *out = filter.process(source);
            }
        }
    }
}
