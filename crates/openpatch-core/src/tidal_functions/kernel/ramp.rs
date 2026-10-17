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
// Adapted from tides2/ramp_generator.h; SHA-256:
// ef737e9bf5041e2c49c1b0731088f7fdcc2c4d59a498035b4bf847b552940fe7
// Source adaptation notice and pinned provenance are in ../kernel.rs.
use super::{cmin, Ratio};

#[derive(Clone, Debug)]
pub(super) struct RampGenerator {
    pub phase: [f32; 4],
    pub frequency: [f32; 4],
    master_phase: f32,
    wrap_counter: [i32; 4],
    ratio: [Ratio; 4],
    pub next_ratio: [Ratio; 4],
}

impl RampGenerator {
    pub fn new() -> Self {
        Self {
            phase: [0.0; 4],
            frequency: [0.0; 4],
            master_phase: 0.0,
            wrap_counter: [0; 4],
            ratio: [Ratio::ONE; 4],
            next_ratio: [Ratio::ONE; 4],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        mode: u8,
        output: u8,
        range: u8,
        f0: f32,
        pw: &[f32],
        gate: u8,
        ramp: Option<f32>,
    ) {
        let n = if output == 3 || (output == 2 && mode == 2) {
            4
        } else {
            1
        };
        if mode == 0 {
            if gate & 2 != 0 {
                self.phase[..n].fill(0.0);
            }
            for i in 0..n {
                self.frequency[i] = cmin(f0 * self.next_ratio[i].ratio, 0.25);
                if let Some(ramp) = ramp {
                    self.phase[i] = ramp * self.next_ratio[i].ratio;
                } else {
                    self.phase[i] += self.frequency[i];
                }
                self.phase[i] = cmin(self.phase[i], 1.0);
            }
        }
        if mode == 2 {
            if output == 2 {
                self.frequency[..n].fill(f0);
            } else {
                for i in 0..n {
                    self.frequency[i] = cmin(f0 * self.next_ratio[i].ratio, 0.25);
                }
            }
            let up = ramp.map_or(gate & 1 != 0, |r| r < 0.5);
            let clip_at = if up { 0.5 } else { 1.0 };
            for i in 0..n {
                if self.phase[i] < 0.5 && !up {
                    self.phase[i] = 0.5;
                } else if self.phase[i] > 0.5 && up {
                    self.phase[i] = 0.0;
                }
                let this_pw = pw[if output == 3 { 0 } else { i }];
                let slope = if self.phase[i] < 0.5 {
                    0.5 / (1.0e-6 + this_pw)
                } else {
                    0.5 / (1.0 + 1.0e-6 - this_pw)
                };
                self.phase[i] += self.frequency[i] * slope;
                self.phase[i] = cmin(self.phase[i], clip_at);
            }
        }
        if mode == 1 {
            if range == 1 && output == 3 {
                let reset = gate & 2 != 0;
                if reset {
                    self.phase[..n].fill(0.0);
                }
                for i in 0..n {
                    self.frequency[i] = cmin(f0 * self.next_ratio[i].ratio, 0.25);
                }
                if !reset {
                    for i in 0..n {
                        self.phase[i] += self.frequency[i];
                        if self.phase[i] >= 1.0 {
                            self.phase[i] -= 1.0;
                        }
                    }
                }
            } else {
                if let Some(ramp) = ramp {
                    for i in 0..n {
                        self.frequency[i] = cmin(f0 * self.ratio[i].ratio, 0.25);
                    }
                    if ramp < self.master_phase {
                        self.wrap(n);
                    }
                    self.master_phase = ramp;
                } else {
                    let reset = gate & 2 != 0;
                    if reset {
                        self.master_phase = 0.0;
                        self.ratio[..n].copy_from_slice(&self.next_ratio[..n]);
                        self.wrap_counter[..n].fill(0);
                    }
                    for i in 0..n {
                        self.frequency[i] = cmin(f0 * self.ratio[i].ratio, 0.25);
                    }
                    if !reset {
                        self.master_phase += f0;
                    }
                    if self.master_phase >= 1.0 {
                        self.master_phase -= 1.0;
                        self.wrap(n);
                    }
                }
                for i in 0..n {
                    let mut mult_phase = self.master_phase + self.wrap_counter[i] as f32;
                    mult_phase *= self.ratio[i].ratio;
                    self.phase[i] = mult_phase - (mult_phase as i32) as f32;
                }
            }
        }
    }

    fn wrap(&mut self, n: usize) {
        for i in 0..n {
            self.wrap_counter[i] += 1;
            if self.wrap_counter[i] >= self.ratio[i].q {
                self.ratio[i] = self.next_ratio[i];
                self.wrap_counter[i] = 0;
            }
        }
    }
}
