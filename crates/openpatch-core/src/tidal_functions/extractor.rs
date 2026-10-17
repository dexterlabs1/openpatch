// Preserve the pinned source arithmetic/decimal constants and update order.
#![allow(clippy::manual_clamp)]

// Copyright 2012 Emilie Gillet.
// Copyright 2017 Emilie Gillet.
// Rust adaptation Copyright 2026 OpenPatch contributors.
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
// Source: pichenettes/eurorack f4cfa7697095a28d292bff758beb51deb49b5a86,
// tides2/ramp/ramp_extractor.{h,cc}. Keep operation order and Reset semantics.

const HIGH: u8 = 1;
const RISING: u8 = 2;
const FALLING: u8 = 4;
const HISTORY_SIZE: usize = 16;
const MAX_PATTERN_PERIOD: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
struct Pulse {
    on_duration: u32,
    total_duration: u32,
    pulse_width: f32,
}

/// Raw portable-source clock tracker. Callers must ensure at least one input
/// sample has elapsed before a rising edge after construction or reset.
/// Startup gate priming belongs to the host adapter, outside this translation.
#[derive(Clone, Debug)]
pub struct RampExtractor {
    current_pulse: usize,
    history: [Pulse; HISTORY_SIZE],
    prediction_error: [f32; MAX_PATTERN_PERIOD + 1],
    predicted_period: [f32; MAX_PATTERN_PERIOD + 1],
    average_pulse_width: f32,
    train_phase: f32,
    frequency_lp: f32,
    frequency: f32,
    target_frequency: f32,
    lp_coefficient: f32,
    period: i32,
    reset_counter: i32,
    #[allow(dead_code)] // Retained upstream state; assigned but unused by Process.
    max_ramp_value: f32,
    f_ratio: f32,
    max_train_phase: f32,
    reset_interval: u32,
    min_period: f32,
    sample_rate: f32,
}

// Match std::min/max operand ordering, including its first-operand NaN rule.
fn minimum(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}
fn maximum(a: f32, b: f32) -> f32 {
    if a < b {
        b
    } else {
        a
    }
}

impl RampExtractor {
    pub fn new(sample_rate: f32, max_frequency: f32) -> Self {
        // Firmware instances have static storage: fields omitted by Init/Reset
        // start at zero. In particular Reset never assigns max_train_phase.
        let mut result = Self {
            current_pulse: 0,
            history: [Pulse::default(); HISTORY_SIZE],
            prediction_error: [0.0; MAX_PATTERN_PERIOD + 1],
            predicted_period: [0.0; MAX_PATTERN_PERIOD + 1],
            average_pulse_width: 0.0,
            train_phase: 0.0,
            frequency_lp: 0.0,
            frequency: 0.0,
            target_frequency: 0.0,
            lp_coefficient: 0.0,
            period: 0,
            reset_counter: 0,
            max_ramp_value: 0.0,
            f_ratio: 0.0,
            max_train_phase: 0.0,
            reset_interval: 0,
            min_period: 1.0 / max_frequency,
            sample_rate,
        };
        result.reset();
        result
    }

    pub fn reset(&mut self) {
        self.train_phase = 0.0;
        self.frequency = 0.1 / self.sample_rate;
        self.frequency_lp = self.frequency;
        self.target_frequency = self.frequency;
        self.period = (1.0 / self.frequency) as i32;
        self.lp_coefficient = 0.1;
        self.max_ramp_value = 1.0;
        self.f_ratio = 1.0;
        self.reset_counter = 1;
        self.reset_interval = (self.sample_rate as u32).wrapping_mul(3);
        self.history.fill(Pulse {
            on_duration: (self.sample_rate * 0.25) as u32,
            total_duration: (self.sample_rate * 0.5) as u32,
            pulse_width: 0.5,
        });
        self.current_pulse = 0;
        self.history[0].on_duration = 0;
        self.history[0].total_duration = 0;
        self.average_pulse_width = 0.0;
        self.prediction_error.fill(50.0);
        self.predicted_period.fill(self.sample_rate * 0.5);
        self.prediction_error[0] = 0.0;
    }

    fn compute_average_pulse_width(&self, tolerance: f32) -> f32 {
        let mut sum = 0.0;
        let y = self.history[self.current_pulse].pulse_width;
        for pulse in &self.history {
            let x = pulse.pulse_width;
            if !(x >= y * (1.0 - tolerance) && x <= y * (1.0 + tolerance)) {
                return 0.0;
            }
            sum += x;
        }
        sum / HISTORY_SIZE as f32
    }

    fn predict_next_period(&mut self) -> f32 {
        let last_period = self.history[self.current_pulse].total_duration as f32;
        let mut best_pattern_period = 0;
        for i in 0..=MAX_PATTERN_PERIOD {
            let error = self.predicted_period[i] - last_period;
            let error_sq = error * error;
            let slope_error = error_sq - self.prediction_error[i];
            let coefficient = if slope_error > 0.0 { 0.7 } else { 0.2 };
            self.prediction_error[i] += coefficient * slope_error;
            if i == 0 {
                self.predicted_period[0] += 0.5 * (last_period - self.predicted_period[0]);
            } else {
                let t = self.current_pulse + 1 + HISTORY_SIZE - i;
                self.predicted_period[i] = self.history[t % HISTORY_SIZE].total_duration as f32;
            }
            if self.prediction_error[i] < self.prediction_error[best_pattern_period] {
                best_pattern_period = i;
            }
        }
        self.predicted_period[best_pattern_period]
    }

    pub fn process(
        &mut self,
        smooth_audio_tracking: bool,
        force_integer_period: bool,
        ratio: f32,
        denominator: i32,
        gates: &[u8],
        ramp: &mut [f32],
    ) -> f32 {
        assert_eq!(gates.len(), ramp.len());
        let block_size = gates.len();
        for (&flags, output) in gates.iter().zip(ramp) {
            if flags & RISING != 0 {
                let total_duration = self.history[self.current_pulse].total_duration;
                let record_pulse = total_duration < self.reset_interval;
                if !record_pulse {
                    self.reset_counter = denominator;
                    self.train_phase = 0.0;
                    self.f_ratio = ratio;
                    self.max_train_phase = denominator as f32;
                    self.reset_interval = total_duration.wrapping_mul(4);
                } else {
                    let period = total_duration as f32;
                    if smooth_audio_tracking {
                        let mut no_glide = self.f_ratio != ratio;
                        self.f_ratio = ratio;
                        self.reset_counter -= 1;
                        let mut phase_error = 0.0;
                        if self.reset_counter == 0 {
                            self.reset_counter = denominator;
                            let mut expected_phase =
                                2.0 * block_size as f32 / period * self.f_ratio;
                            while expected_phase >= 1.0 {
                                expected_phase -= 1.0;
                            }
                            phase_error = self.train_phase - expected_phase;
                            if phase_error > 0.5 {
                                phase_error -= 1.0;
                            }
                            if phase_error < -0.5 {
                                phase_error += 1.0;
                            }
                        }
                        let frequency = 1.0 / period;
                        let mut pll_adjustment =
                            1.0 - self.lp_coefficient * phase_error / self.f_ratio;
                        if pll_adjustment < 0.99 {
                            pll_adjustment = 0.99;
                        }
                        if pll_adjustment > 1.01 {
                            pll_adjustment = 1.01;
                        }
                        self.target_frequency =
                            minimum(self.f_ratio * frequency * pll_adjustment, 0.125);
                        let up_tolerance = (1.02 + 2.0 * frequency) * self.frequency_lp;
                        let down_tolerance = (0.98 - 2.0 * frequency) * self.frequency_lp;
                        no_glide |= self.target_frequency > up_tolerance
                            || self.target_frequency < down_tolerance;
                        self.lp_coefficient = if no_glide {
                            1.0
                        } else {
                            minimum(period * 0.00001, 0.1)
                        };
                    } else {
                        if period < self.min_period {
                            self.target_frequency = 1.0 / period;
                            self.frequency = self.target_frequency;
                        } else {
                            let pulse = &mut self.history[self.current_pulse];
                            pulse.pulse_width =
                                pulse.on_duration as f32 / pulse.total_duration as f32;
                            self.average_pulse_width = self.compute_average_pulse_width(0.05);
                            if self.history[self.current_pulse].on_duration < 32 {
                                self.average_pulse_width = 0.0;
                            }
                            self.target_frequency = 1.0 / self.predict_next_period();
                            self.frequency = self.target_frequency;
                        }
                        self.reset_counter -= 1;
                        if self.reset_counter == 0 {
                            self.train_phase = 0.0;
                            self.reset_counter = denominator;
                            self.f_ratio = ratio;
                            self.max_train_phase = denominator as f32;
                        } else {
                            let expected = self.max_train_phase - self.reset_counter as f32;
                            let warp = expected - self.train_phase + 1.0;
                            self.frequency *= maximum(warp, 0.01);
                        }
                    }
                    self.reset_interval =
                        maximum(4.0 / self.target_frequency, self.sample_rate * 3.0) as u32;
                    self.current_pulse = (self.current_pulse + 1) % HISTORY_SIZE;
                }
                self.history[self.current_pulse].on_duration = 0;
                self.history[self.current_pulse].total_duration = 0;
            }
            let pulse = &mut self.history[self.current_pulse];
            pulse.total_duration = pulse.total_duration.wrapping_add(1);
            if flags & HIGH != 0 {
                pulse.on_duration = pulse.on_duration.wrapping_add(1);
            }
            if smooth_audio_tracking {
                self.frequency_lp +=
                    self.lp_coefficient * (self.target_frequency - self.frequency_lp);
                if force_integer_period {
                    let new_period = (1.0 / self.frequency_lp) as i32;
                    if (new_period - self.period).abs() > 1 {
                        self.period = new_period;
                        self.frequency = 1.0 / new_period as f32;
                    }
                } else {
                    self.frequency = self.frequency_lp;
                }
                self.train_phase += self.frequency;
                if self.train_phase >= 1.0 {
                    self.train_phase -= 1.0;
                }
                *output = self.train_phase;
            } else {
                if flags & FALLING != 0 && self.average_pulse_width > 0.0 {
                    let t_on = self.history[self.current_pulse].on_duration as f32;
                    let next = self.max_train_phase - self.reset_counter as f32 + 1.0;
                    let pw = self.average_pulse_width;
                    self.frequency =
                        maximum(next - self.train_phase, 0.0) * pw / ((1.0 - pw) * t_on);
                }
                self.train_phase += self.frequency;
                if self.train_phase >= self.max_train_phase {
                    self.train_phase = self.max_train_phase;
                }
                let mut phase = self.train_phase * self.f_ratio;
                phase -= (phase as i32) as f32;
                *output = phase;
            }
        }
        if smooth_audio_tracking {
            self.frequency
        } else {
            self.frequency * self.f_ratio
        }
    }
}
