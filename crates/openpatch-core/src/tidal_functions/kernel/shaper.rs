// Preserve the pinned source arithmetic/decimal constants and update order.
#![allow(clippy::assign_op_pattern)]

// Copyright 2012 Emilie Gillet.
// Copyright 2017 Emilie Gillet.
//
// Author: Emilie Gillet (emilie.o.gillet@gmail.com)
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to enable, copy, modify, merge, publish, distribute, sublicense, and/or sell
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
// Adapted from tides2/ramp_shaper.h; SHA-256:
// da7c348b7f7bc46c1892854d27d82f750c37aa85edebe6f02792811ef60a0e46
// Source adaptation notice and pinned provenance are in ../kernel.rs.
use super::{cmin, constrain, resources::LUT_WAVETABLE};

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RampShaper {
    next_sample: f32,
    previous_phase_shift: f32,
}

fn this_blep(t: f32) -> f32 {
    0.5 * t * t
}
fn next_blep(t: f32) -> f32 {
    let t = 1.0 - t;
    -0.5 * t * t
}
fn next_integrated_blep(t: f32) -> f32 {
    let t1 = 0.5 * t;
    let t2 = t1 * t1;
    let t4 = t2 * t2;
    0.1875 - t1 + 1.5 * t2 - t4
}

impl RampShaper {
    fn pulse(&mut self, phase: f32, frequency: f32, mut pw: f32) -> f32 {
        constrain(&mut pw, frequency * 2.0, 1.0 - 2.0 * frequency);
        let mut this_sample = self.next_sample;
        let mut next_sample = 0.0;
        let wrap_point = if phase < pw * 0.5 {
            0.0
        } else if phase > 0.5 + pw * 0.5 {
            1.0
        } else {
            pw
        };
        let d = phase - wrap_point;
        if d >= 0.0 && d < frequency {
            let t = d / frequency;
            let mut discontinuity = 1.0;
            if wrap_point != pw {
                discontinuity = -discontinuity;
            }
            if frequency < 0.0 {
                discontinuity = -discontinuity;
            }
            this_sample += this_blep(t) * discontinuity;
            next_sample += next_blep(t) * discontinuity;
        }
        next_sample += if phase < pw { 0.0 } else { 1.0 };
        self.next_sample = next_sample;
        this_sample
    }

    pub fn slope(
        &mut self,
        mode: u8,
        range: u8,
        phase: f32,
        phase_shift: f32,
        frequency: f32,
        pw: f32,
    ) -> f32 {
        if mode == 0 {
            self.skewed(phase, 0.0, frequency, pw)
        } else if mode == 2 {
            phase
        } else if range == 0 {
            self.skewed(phase, phase_shift, frequency, pw)
        } else {
            self.band_limited(phase, phase_shift, frequency, pw)
        }
    }

    pub fn eoa(&mut self, mode: u8, range: u8, phase: f32, frequency: f32, pw: f32) -> f32 {
        if mode == 1 && range == 1 {
            self.pulse(phase, frequency, pw)
        } else if mode == 2 {
            (phase >= 0.5) as u8 as f32
        } else {
            (phase >= pw) as u8 as f32
        }
    }

    pub fn eor(&mut self, mode: u8, range: u8, phase: f32, frequency: f32) -> f32 {
        if mode == 1 {
            let pw = cmin(0.5, 96.0 * frequency);
            if range == 1 {
                1.0 - self.pulse(phase, frequency, pw)
            } else {
                (phase < pw) as u8 as f32
            }
        } else {
            (phase >= 1.0) as u8 as f32
        }
    }

    fn shift(&mut self, phase: &mut f32, frequency: &mut f32, phase_shift: f32) {
        if phase_shift != 0.0 {
            *phase += phase_shift;
            *frequency += phase_shift - self.previous_phase_shift;
            self.previous_phase_shift = phase_shift;
            if *phase >= 1.0 {
                *phase -= 1.0;
            } else if *phase < 0.0 {
                *phase += 1.0;
            }
        }
    }

    fn skewed(&mut self, mut phase: f32, phase_shift: f32, mut frequency: f32, mut pw: f32) -> f32 {
        self.shift(&mut phase, &mut frequency, phase_shift);
        constrain(&mut pw, frequency.abs() * 2.0, 1.0 - 2.0 * frequency.abs());
        let slope_up = 0.5 / pw;
        let slope_down = 0.5 / (1.0 - pw);
        if phase < pw {
            phase * slope_up
        } else {
            (phase - pw) * slope_down + 0.5
        }
    }

    fn band_limited(
        &mut self,
        mut phase: f32,
        phase_shift: f32,
        mut frequency: f32,
        mut pw: f32,
    ) -> f32 {
        self.shift(&mut phase, &mut frequency, phase_shift);
        constrain(&mut pw, frequency.abs() * 2.0, 1.0 - 2.0 * frequency.abs());
        let mut this_sample = self.next_sample;
        let mut next_sample = 0.0;
        let wrap_point = if phase < pw * 0.5 {
            0.0
        } else if phase > 0.5 + pw * 0.5 {
            1.0
        } else {
            pw
        };
        let slope_up = 1.0 / pw;
        let slope_down = 1.0 / (1.0 - pw);
        let d = phase - wrap_point;
        if d >= 0.0 && d < frequency {
            let t = d / frequency;
            let mut discontinuity = -(slope_up + slope_down) * frequency;
            if wrap_point != pw {
                discontinuity = -discontinuity;
            }
            if frequency < 0.0 {
                discontinuity = -discontinuity;
            }
            this_sample += next_integrated_blep(1.0 - t) * discontinuity;
            next_sample += next_integrated_blep(t) * discontinuity;
        }
        next_sample += if phase < pw {
            phase * slope_up
        } else {
            1.0 - (phase - pw) * slope_down
        };
        self.next_sample = next_sample;
        this_sample
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RampWaveshaper {
    previous_input: f32,
    previous_output: f32,
    breakpoint: f32,
}

impl RampWaveshaper {
    pub fn shape(&mut self, mode: u8, input: f32, table: usize, shape_fractional: f32) -> f32 {
        let ws_index = 1024.0 * input;
        let integral = ws_index as i32;
        let fractional = ws_index - integral as f32;
        let index = (integral & 1023) as usize + table;
        let x0 = LUT_WAVETABLE[index] as f32 / 32768.0;
        let x1 = LUT_WAVETABLE[index + 1] as f32 / 32768.0;
        let y0 = LUT_WAVETABLE[index + 1025] as f32 / 32768.0;
        let y1 = LUT_WAVETABLE[index + 1026] as f32 / 32768.0;
        let x = x0 + (x1 - x0) * fractional;
        let y = y0 + (y1 - y0) * fractional;
        let mut output = x + (y - x) * shape_fractional;
        if mode == 2 {
            if (self.previous_input <= 0.5 && input > 0.5)
                || (self.previous_input > 0.5 && input < 0.5)
            {
                self.breakpoint = self.previous_output;
            } else if input == 1.0 {
                self.breakpoint = 1.0;
            } else if input == 0.5 {
                self.breakpoint = 0.0;
            }
            if input <= 0.5 {
                output = self.breakpoint + (1.0 - self.breakpoint) * output;
            } else {
                output = self.breakpoint * output;
            }
            self.previous_input = input;
            self.previous_output = output;
        }
        output
    }
}
