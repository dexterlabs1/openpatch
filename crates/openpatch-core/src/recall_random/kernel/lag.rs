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
// Source: Marbles random/lag_processor.{h,cc}.
use super::math::{interpolate, semitones_to_ratio};
use super::resources::LUT_RAISED_COSINE;

#[derive(Clone)]
pub struct LagProcessor {
    ramp_start: f32,
    ramp_value: f32,
    lp_state: f32,
    previous_phase: f32,
}
impl LagProcessor {
    pub fn new() -> Self {
        Self {
            ramp_start: 0.0,
            ramp_value: 0.0,
            lp_state: 0.0,
            previous_phase: 0.0,
        }
    }
    pub fn reset_ramp(&mut self) {
        self.ramp_start = self.ramp_value;
    }
    pub fn process(&mut self, value: f32, smoothness: f32, phase: f32) -> f32 {
        let mut frequency = phase - self.previous_phase;
        if frequency < 0.0 {
            frequency += 1.0;
        }
        self.previous_phase = phase;
        frequency *= 0.25;
        frequency *= semitones_to_ratio(84.0 * (1.0 - smoothness));
        if frequency >= 1.0 {
            frequency = 1.0;
        }
        if smoothness <= 0.05 {
            frequency += 20.0 * (0.05 - smoothness) * (1.0 - frequency);
        }
        self.lp_state += frequency * (value - self.lp_state);
        let interp_amount = ((smoothness - 0.6) * 5.0).clamp(0.0, 1.0);
        let interp_linearity = ((1.0 - smoothness) * 5.0).clamp(0.0, 1.0);
        let warped_phase = interpolate(&LUT_RAISED_COSINE, phase, 256.0);
        let interp_phase = warped_phase + (phase - warped_phase) * interp_linearity;
        let interp = self.ramp_start + (value - self.ramp_start) * interp_phase;
        self.ramp_value = interp;
        self.lp_state + (interp - self.lp_state) * interp_amount
    }
}
