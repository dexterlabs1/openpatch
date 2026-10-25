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
// Rust adaptation (OpenPatch Grain Weather) of clouds/dsp/audio_buffer.h and
// clouds/dsp/mu_law.h at eurorack f4cfa7697095a28d292bff758beb51deb49b5a86.
// The recording buffer lives in the preallocated memory image; the source's
// two template instantiations (16-bit, 8-bit mu-law) become a runtime flag.

use super::memory::Memory;
use super::resources::LUT_ULAW;
use super::stmlib::{clip16, trunc_i32};

pub(super) const CROSS_FADE_SIZE: i32 = 256;
pub(super) const INTERPOLATION_TAIL: i32 = 8;

#[inline]
pub(super) fn mu_law_to_linear(u: u8) -> i16 {
    LUT_ULAW[u as usize]
}

/// `Lin2MuLaw`, statement for statement.
#[inline]
pub(super) fn linear_to_mu_law(pcm: i16) -> u8 {
    let mut pcm_val = pcm >> 2;
    let mask: i16;
    if pcm_val < 0 {
        pcm_val = -pcm_val;
        mask = 0x7f;
    } else {
        mask = 0xff;
    }
    if pcm_val > 8159 {
        pcm_val = 8159;
    }
    pcm_val += 0x84 >> 2;
    let seg: i16 = if pcm_val <= 0x3f {
        0
    } else if pcm_val <= 0x7f {
        1
    } else if pcm_val <= 0xff {
        2
    } else if pcm_val <= 0x1ff {
        3
    } else if pcm_val <= 0x3ff {
        4
    } else if pcm_val <= 0x7ff {
        5
    } else if pcm_val <= 0xfff {
        6
    } else if pcm_val <= 0x1fff {
        7
    } else {
        8
    };
    if seg >= 8 {
        (0x7f ^ mask) as u8
    } else {
        let uval = ((seg << 4) | ((pcm_val >> (seg + 1)) & 0x0f)) as u8;
        uval ^ (mask as u8)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AudioBuffer {
    base: usize,
    mu_law: bool,
    size: i32,
    write_head: i32,
    crossfade_counter: i32,
}

impl AudioBuffer {
    /// `Init(buffer, size, tail)`: `size` is in samples (bytes for mu-law).
    pub(super) fn init(&mut self, memory: &mut Memory, base: usize, size: i32, mu_law: bool) {
        self.base = base;
        self.mu_law = mu_law;
        self.size = size - INTERPOLATION_TAIL;
        self.write_head = 0;
        self.crossfade_counter = 0;
        if mu_law {
            memory.fill_u8(base, size as usize, 127);
        } else {
            memory.fill_u16(base, size as usize, 0);
        }
    }

    #[inline]
    pub(super) fn size(&self) -> i32 {
        self.size
    }

    #[inline]
    pub(super) fn head(&self) -> i32 {
        self.write_head
    }

    #[inline]
    fn store(&self, memory: &mut Memory, index: i32, sample: i32) {
        if self.mu_law {
            memory.set_u8(
                self.base.wrapping_add(index as usize),
                linear_to_mu_law(sample as i16),
            );
        } else {
            memory.set_i16(self.base.wrapping_add(2 * index as usize), sample as i16);
        }
    }

    #[inline]
    fn copy_cell(&self, memory: &mut Memory, from: i32, to: i32) {
        if self.mu_law {
            let value = memory.u8(self.base + from as usize);
            memory.set_u8(self.base + to as usize, value);
        } else {
            let value = memory.u16(self.base + 2 * from as usize);
            memory.set_u16(self.base + 2 * to as usize, value);
        }
    }

    #[inline]
    fn write(&mut self, memory: &mut Memory, input: f32) {
        // Both resolutions quantize to 16 bits first; mu-law then compresses.
        let sample = clip16(trunc_i32(input * 32768.0));
        self.store(memory, self.write_head, sample);
        if self.write_head < INTERPOLATION_TAIL {
            self.copy_cell(memory, self.write_head, self.write_head + self.size);
        }
        self.write_head += 1;
        if self.write_head >= self.size {
            self.write_head = 0;
        }
    }

    /// `WriteFade` over one channel of interleaved frames.
    pub(super) fn write_fade(
        &mut self,
        memory: &mut Memory,
        tail: &mut [i16; 256],
        input: &[[f32; 2]],
        channel: usize,
        write: bool,
    ) {
        let size = input.len() as i32;
        if !write {
            // Continue recording samples to have something to crossfade
            // with when recording resumes.
            let mut source = input.iter();
            if self.crossfade_counter < CROSS_FADE_SIZE {
                for _ in 0..size {
                    if self.crossfade_counter < CROSS_FADE_SIZE {
                        let x = source.next().map_or(0.0, |frame| frame[channel]);
                        tail[self.crossfade_counter as usize] =
                            clip16(trunc_i32(x * 32767.0)) as i16;
                        self.crossfade_counter += 1;
                    }
                }
            }
        } else if self.crossfade_counter == 0
            && !self.mu_law
            && self.write_head >= INTERPOLATION_TAIL
            && self.write_head < (self.size - size)
        {
            // Fast write routine for the most common case.
            for frame in input {
                let sample = clip16(trunc_i32(frame[channel] * 32767.0));
                memory.set_i16(self.base + 2 * self.write_head as usize, sample as i16);
                self.write_head += 1;
            }
        } else {
            for frame in input {
                let mut sample = frame[channel];
                if self.crossfade_counter != 0 {
                    self.crossfade_counter -= 1;
                    // At counter zero the source reads one element past the
                    // tail, scaled by a zero gain: the sample is unchanged.
                    let tail_sample = tail
                        .get((CROSS_FADE_SIZE - self.crossfade_counter) as usize)
                        .map_or(0.0, |&value| value as f32);
                    let gain = self.crossfade_counter as f32 * (1.0 / CROSS_FADE_SIZE as f32);
                    sample += (tail_sample / 32768.0 - sample) * gain;
                }
                self.write(memory, sample);
            }
        }
    }

    #[inline]
    fn sample(&self, memory: &Memory, index: i32) -> f32 {
        if index < 0 || index >= self.size + INTERPOLATION_TAIL {
            // Outside this buffer: the source reads neighbouring memory.
            memory.guarded.set(memory.guarded.get() + 1);
        }
        if self.mu_law {
            mu_law_to_linear(memory.u8(self.base.wrapping_add(index as isize as usize))) as f32
        } else {
            memory.i16(self.base.wrapping_add((2 * index as isize) as usize)) as f32
        }
    }

    #[inline]
    pub(super) fn read_zoh(&self, memory: &Memory, mut integral: i32, _fractional: u16) -> f32 {
        if integral >= self.size {
            integral -= self.size;
        }
        self.sample(memory, integral) * (1.0 / 32768.0)
    }

    #[inline]
    pub(super) fn read_linear(&self, memory: &Memory, mut integral: i32, fractional: u16) -> f32 {
        if integral >= self.size {
            integral -= self.size;
        }
        let t = fractional as f32 / 65536.0;
        let x0 = self.sample(memory, integral);
        let x1 = self.sample(memory, integral + 1);
        (x0 + (x1 - x0) * t) * (1.0 / 32768.0)
    }

    #[inline]
    pub(super) fn read_hermite(&self, memory: &Memory, mut integral: i32, fractional: u16) -> f32 {
        if integral >= self.size {
            integral -= self.size;
        }
        let t = fractional as f32 / 65536.0;
        let xm1 = self.sample(memory, integral);
        let x0 = self.sample(memory, integral + 1);
        let x1 = self.sample(memory, integral + 2);
        let x2 = self.sample(memory, integral + 3);
        // Laurent de Soras's Hermite interpolator.
        let c = (x1 - xm1) * 0.5;
        let v = x0 - x1;
        let w = c + v;
        let a = w + v + (x2 - x0) * 0.5;
        let b_neg = w + a;
        ((((a * t) - b_neg) * t + c) * t + x0) * (1.0 / 32768.0)
    }
}
