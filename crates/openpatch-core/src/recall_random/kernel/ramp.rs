// Copyright 2015 Emilie Gillet. MIT license; see crate LICENSE-THIRD-PARTY.
// Source-derived Marbles ramp_generator/slave_ramp/ramp_divider/ramp_extractor.
use super::Ratio;
const MAX_RAMP: f32 = 0.9999;
fn ratio_float(r: Ratio) -> f32 {
    r.p as f32 / r.q as f32
}

#[derive(Clone, Debug, Default)]
pub struct RampGenerator {
    phase: f32,
}
impl RampGenerator {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn render(&mut self, frequency: f32, out: &mut [f32]) {
        for value in out {
            self.phase += frequency;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            *value = self.phase;
        }
    }
}

#[derive(Clone, Debug)]
pub struct SlaveRamp {
    phase: f32,
    max_phase: f32,
    ratio: f32,
    pulse_width: f32,
    target: f32,
    pulse_length: i32,
    bernoulli: bool,
    must_complete: bool,
}
impl SlaveRamp {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            max_phase: MAX_RAMP,
            ratio: 1.0,
            pulse_width: 0.0,
            target: 1.0,
            pulse_length: 0,
            bernoulli: false,
            must_complete: false,
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new();
        self.phase = 1.0;
    }
    pub fn init_pattern(&mut self, length: i32, ratio: Ratio, pulse_width: f32) {
        self.bernoulli = false;
        self.phase = 0.0;
        self.max_phase = length as f32 * MAX_RAMP;
        self.ratio = ratio_float(ratio);
        self.pulse_width = pulse_width;
        self.target = 1.0;
        self.pulse_length = 0;
    }
    pub fn init_bernoulli(&mut self, must_complete: bool, pulse_width: f32, expected_value: f32) {
        self.bernoulli = true;
        if self.must_complete {
            self.phase = 0.0;
            self.pulse_width = pulse_width;
            self.ratio = 1.0;
            self.pulse_length = 0;
        }
        self.ratio = if !must_complete {
            (1.0 - self.phase) * expected_value
        } else {
            1.0 - self.phase
        };
        self.must_complete = must_complete;
    }
    pub fn process(&mut self, frequency: f32) -> (f32, bool) {
        let mut output_phase;
        if self.bernoulli {
            self.phase += frequency * self.ratio;
            output_phase = self.phase;
            if output_phase >= 1.0 {
                output_phase = 1.0;
            }
        } else {
            self.phase += frequency;
            if self.phase >= self.max_phase {
                self.phase = self.max_phase;
            }
            output_phase = self.phase * self.ratio;
            if output_phase > self.target {
                self.pulse_length = 0;
                self.target += 1.0;
            }
            output_phase -= output_phase as i32 as f32;
        }
        let gate = if self.pulse_width == 0.0 {
            self.pulse_length < 32 && output_phase <= 0.5
        } else {
            output_phase < self.pulse_width
        };
        // Explicit wrapping extends source signed counter overflow in long runs.
        self.pulse_length = self.pulse_length.wrapping_add(1);
        (output_phase, gate)
    }
}

#[derive(Clone, Debug)]
pub struct RampDivider {
    phase: f32,
    train_phase: f32,
    max_train_phase: f32,
    f_ratio: f32,
    reset_counter: i32,
    reset_at_next_pulse: bool,
}
impl RampDivider {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            train_phase: 0.0,
            max_train_phase: 1.0,
            f_ratio: 0.99999,
            reset_counter: 1,
            reset_at_next_pulse: false,
        }
    } // C++ global zero initialization.
    pub fn reset(&mut self) {
        self.reset_at_next_pulse = true;
    }
    pub fn process(&mut self, ratio: Ratio, input: &[f32], out: &mut [f32]) {
        for (&new_phase, value) in input.iter().zip(out) {
            let mut frequency = new_phase - self.phase;
            if frequency < 0.0 {
                if self.reset_at_next_pulse {
                    self.reset_at_next_pulse = false;
                    self.reset_counter = 1;
                }
                frequency += 1.0;
                self.reset_counter -= 1;
                if self.reset_counter == 0 {
                    self.train_phase = new_phase;
                    self.reset_counter = ratio.q;
                    self.f_ratio = ratio_float(ratio) * MAX_RAMP;
                    frequency = 0.0;
                    self.max_train_phase = ratio.q as f32;
                }
            }
            self.train_phase += frequency;
            if self.train_phase >= self.max_train_phase {
                self.train_phase = self.max_train_phase;
            }
            let mut output_phase = self.train_phase * self.f_ratio;
            output_phase -= output_phase as i32 as f32;
            *value = output_phase;
            self.phase = new_phase;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Pulse {
    on_duration: u32,
    total_duration: u32,
    bucket: u32,
    pulse_width: f32,
}
#[derive(Clone, Debug)]
pub struct RampExtractor {
    current_pulse: usize,
    history: [Pulse; 16],
    next_bucket: f32,
    prediction_hash_table: [f32; 256],
    predicted_period: [f32; 13],
    prediction_accuracy: [f32; 13],
    average_pulse_width: f32,
    train_phase: f32,
    frequency: f32,
    max_train_phase: f32,
    reset_frequency: f32,
    target_frequency: f32,
    lp_coefficient: f32,
    f_ratio: f32,
    next_f_ratio: f32,
    next_max_train_phase: f32,
    reset_counter: i32,
    reset_interval: u32,
    audio_rate: bool,
    num_consistent_audio_rate_pulses: i32,
    max_frequency: f32,
    audio_rate_period: f32,
    audio_rate_period_hysteresis: f32,
    reset_at_next_pulse: bool,
}
fn within(x: f32, y: f32, error: f32) -> bool {
    x >= y * (1.0 - error) && x <= y * (1.0 + error)
}
impl RampExtractor {
    pub fn new(max_frequency: f32) -> Self {
        let period = 1.0 / (100.0 / 32000.0);
        let mut result = Self {
            current_pulse: 0,
            history: [Pulse {
                on_duration: 0,
                total_duration: 0,
                bucket: 0,
                pulse_width: 0.0,
            }; 16],
            next_bucket: 0.0,
            prediction_hash_table: [0.0; 256],
            predicted_period: [0.0; 13],
            prediction_accuracy: [0.0; 13],
            average_pulse_width: 0.0,
            train_phase: 0.0,
            frequency: 0.0,
            max_train_phase: 0.0,
            reset_frequency: 0.0,
            target_frequency: 0.0,
            lp_coefficient: 0.0,
            f_ratio: 0.0,
            next_f_ratio: 0.0,
            next_max_train_phase: 0.0,
            reset_counter: 0,
            reset_interval: 0,
            audio_rate: false,
            num_consistent_audio_rate_pulses: 0,
            max_frequency,
            audio_rate_period: period,
            audio_rate_period_hysteresis: period,
            reset_at_next_pulse: false,
        };
        result.reset();
        result
    }
    pub fn reset(&mut self) {
        self.audio_rate = false;
        self.num_consistent_audio_rate_pulses = 0;
        self.train_phase = 0.0;
        self.frequency = 0.0001;
        self.target_frequency = 0.0001;
        self.lp_coefficient = 0.5;
        self.max_train_phase = 0.999;
        self.next_max_train_phase = 0.999;
        self.f_ratio = 1.0;
        self.next_f_ratio = 1.0;
        self.reset_counter = 1;
        self.reset_frequency = 0.0;
        self.reset_interval = 32000 * 3;
        self.reset_at_next_pulse = false;
        self.history.fill(Pulse {
            bucket: 1,
            on_duration: 2000,
            total_duration: 4000,
            pulse_width: 0.5,
        });
        self.current_pulse = 0;
        self.next_bucket = 48.0;
        self.average_pulse_width = 0.0;
        self.predicted_period.fill(4000.0);
        self.prediction_accuracy.fill(0.0);
        self.prediction_hash_table.fill(0.0);
    }
    fn compute_average_pulse_width(&self, tolerance: f32) -> f32 {
        let mut sum = 0.0;
        for pulse in &self.history {
            if !within(
                pulse.pulse_width,
                self.history[self.current_pulse].pulse_width,
                tolerance,
            ) {
                return 0.0;
            }
            sum += pulse.pulse_width;
        }
        sum / 16.0
    }
    fn predict_next_period(&mut self) -> (f32, f32) {
        let last_period = self.history[self.current_pulse].total_duration as f32;
        let mut best = 1;
        // Source intentionally starts at FAST; SLOW predictor is never visited.
        for i in 1..13 {
            let error = (self.predicted_period[i] - last_period) / (last_period + 0.01);
            let accuracy = 1.0 / (1.0 + 100.0 * error * error);
            let delta = accuracy - self.prediction_accuracy[i];
            self.prediction_accuracy[i] += if delta > 0.0 {
                0.1 * delta
            } else {
                0.5 * delta
            };
            match i {
                1 => self.predicted_period[i] += 0.5 * (last_period - self.predicted_period[i]),
                2 => {
                    let t2 = (self.current_pulse + 16 - 2) % 16;
                    let t1 = (self.current_pulse + 16 - 1) % 16;
                    let hash = self.history[t1]
                        .bucket
                        .wrapping_add(17u32.wrapping_mul(self.history[t2].bucket))
                        as usize;
                    let h = hash % 256;
                    self.prediction_hash_table[h] +=
                        0.5 * (last_period - self.prediction_hash_table[h]);
                    let hash = self.history[self.current_pulse]
                        .bucket
                        .wrapping_add(17u32.wrapping_mul(self.history[t1].bucket))
                        as usize;
                    self.predicted_period[i] = self.prediction_hash_table[hash % 256];
                    if self.predicted_period[i] == 0.0 {
                        self.predicted_period[i] = last_period;
                    }
                }
                _ => {
                    let candidate_period = i - 3 + 1;
                    let t = self.current_pulse + 1 + 16 - candidate_period;
                    self.predicted_period[i] = self.history[t % 16].total_duration as f32;
                }
            }
            if self.prediction_accuracy[i] >= self.prediction_accuracy[best] {
                best = i;
            }
        }
        (self.predicted_period[best], self.prediction_accuracy[best])
    }
    pub fn process(
        &mut self,
        ratio: Ratio,
        always_ramp_to_maximum: bool,
        reset: &mut bool,
        gate_flags: &[u8],
        ramp: &mut [f32],
    ) {
        if *reset {
            self.reset_at_next_pulse = true;
        }
        for (&flags, out) in gate_flags.iter().zip(ramp) {
            if flags & 2 != 0 {
                let p = self.history[self.current_pulse];
                let record_pulse = p.total_duration < self.reset_interval;
                if !record_pulse {
                    self.reset_frequency = 0.0;
                    self.train_phase = 0.0;
                    self.reset_counter = ratio.q;
                    self.reset_interval = 4u32.wrapping_mul(p.total_duration);
                    *reset = true;
                } else {
                    if self.reset_at_next_pulse {
                        self.reset_counter = 1;
                        self.reset_at_next_pulse = false;
                    }
                    let period = p.total_duration as f32;
                    if period <= self.audio_rate_period_hysteresis {
                        self.num_consistent_audio_rate_pulses =
                            (self.num_consistent_audio_rate_pulses + 1).min(3);
                        self.audio_rate_period_hysteresis = self.audio_rate_period * 1.1;
                    } else {
                        self.num_consistent_audio_rate_pulses = 0;
                        self.audio_rate_period_hysteresis = self.audio_rate_period;
                    }
                    self.audio_rate = self.num_consistent_audio_rate_pulses == 3;
                    if self.audio_rate {
                        self.average_pulse_width = 0.0;
                        let mut no_glide = self.f_ratio != ratio_float(ratio);
                        self.f_ratio = ratio_float(ratio);
                        let frequency = 1.0 / period;
                        self.target_frequency = (self.f_ratio * frequency).min(self.max_frequency);
                        let up_tolerance = (1.02 + 2.0 * frequency) * self.frequency;
                        let down_tolerance = (0.98 - 2.0 * frequency) * self.frequency;
                        no_glide |= self.target_frequency > up_tolerance
                            || self.target_frequency < down_tolerance;
                        self.lp_coefficient = if no_glide {
                            1.0
                        } else {
                            (period * 0.00001).min(0.1)
                        };
                    } else {
                        self.history[self.current_pulse].pulse_width =
                            p.on_duration as f32 / period;
                        self.average_pulse_width = self.compute_average_pulse_width(0.05);
                        if p.on_duration < 32 {
                            self.average_pulse_width = 0.0;
                        }
                        let (period, _accuracy) = self.predict_next_period();
                        self.frequency = 1.0 / period;
                        self.reset_counter -= 1;
                        if self.reset_counter == 0 {
                            self.next_f_ratio = ratio_float(ratio) * MAX_RAMP;
                            self.next_max_train_phase = ratio.q as f32;
                            if always_ramp_to_maximum && self.train_phase < self.max_train_phase {
                                self.reset_frequency =
                                    (0.01 + self.max_train_phase - self.train_phase) * 0.0625;
                            } else {
                                self.reset_frequency = 0.0;
                                self.train_phase = 0.0;
                                self.f_ratio = self.next_f_ratio;
                                self.max_train_phase = self.next_max_train_phase;
                            }
                            self.reset_counter = ratio.q;
                        } else {
                            let expected = self.max_train_phase - self.reset_counter as f32;
                            let warp = expected - self.train_phase + 1.0;
                            self.frequency *= warp.max(0.01);
                        }
                    }
                    // Native saturating float conversion extends C++ UB at +inf.
                    self.reset_interval = (4.0 / self.target_frequency).max(32000.0 * 3.0) as u32;
                    self.current_pulse = (self.current_pulse + 1) % 16;
                }
                self.history[self.current_pulse].on_duration = 0;
                self.history[self.current_pulse].total_duration = 0;
                self.history[self.current_pulse].bucket = 0;
                self.next_bucket = 48.0;
            }
            let p = &mut self.history[self.current_pulse];
            p.total_duration = p.total_duration.wrapping_add(1);
            if flags & 1 != 0 {
                p.on_duration = p.on_duration.wrapping_add(1);
            }
            if p.total_duration as f32 >= self.next_bucket {
                p.bucket = p.bucket.wrapping_add(1);
                self.next_bucket *= f32::from_bits(0x3f9837f0); // source kLogOneFourth
            }
            if flags & 4 != 0 && self.average_pulse_width > 0.0 {
                let t_on = self.history[self.current_pulse].on_duration as f32;
                let next = self.max_train_phase - self.reset_counter as f32 + 1.0;
                let pw = self.average_pulse_width;
                self.frequency = (next - self.train_phase).max(0.0) * pw / ((1.0 - pw) * t_on);
            }
            if self.audio_rate {
                self.frequency += self.lp_coefficient * (self.target_frequency - self.frequency);
                self.train_phase += self.frequency;
                if self.train_phase >= 1.0 {
                    self.train_phase -= 1.0;
                }
                *out = self.train_phase;
            } else {
                if self.reset_frequency != 0.0 {
                    self.train_phase += self.reset_frequency;
                    if self.train_phase >= self.max_train_phase {
                        self.train_phase = 0.0;
                        self.reset_frequency = 0.0;
                        self.f_ratio = self.next_f_ratio;
                        self.max_train_phase = self.next_max_train_phase;
                    }
                } else {
                    self.train_phase += self.frequency;
                    if self.train_phase >= self.max_train_phase {
                        if self.frequency == self.max_frequency {
                            self.train_phase -= self.max_train_phase;
                        } else {
                            self.train_phase = self.max_train_phase;
                        }
                    }
                }
                let mut output_phase = self.train_phase * self.f_ratio;
                output_phase -= output_phase as i32 as f32;
                *out = output_phase;
            }
        }
    }
}
