// Copyright 2015 Emilie Gillet. MIT license; see crate LICENSE-THIRD-PARTY.
// Source-derived Marbles random/t_generator.{h,cc}, all seven source models.
use super::math::{
    beta_distribution_sample, fast_beta_distribution_sample, semitones_to_ratio,
    HysteresisQuantizer2,
};
use super::ramp::{RampExtractor, RampGenerator, SlaveRamp};
use super::random::{RandomSequence, RandomStream};
use super::resources::LUT_LOGIT;
use super::{Ramps, Ratio};
#[derive(Clone, Copy, Debug)]
struct DividerPattern {
    ratios: [Ratio; 2],
    length: i32,
}
const DIVIDER_PATTERNS: [DividerPattern; 17] = [
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 1, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 2, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 2 }, Ratio { p: 1, q: 1 }],
        length: 2,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 4, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 2 }, Ratio { p: 2, q: 1 }],
        length: 2,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 3, q: 2 }],
        length: 2,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 4 }, Ratio { p: 4, q: 1 }],
        length: 4,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 4 }, Ratio { p: 2, q: 1 }],
        length: 4,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 2 }, Ratio { p: 3, q: 2 }],
        length: 2,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 8, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 3, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 3 }, Ratio { p: 1, q: 1 }],
        length: 3,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 5, q: 4 }],
        length: 4,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 2 }, Ratio { p: 5, q: 4 }],
        length: 4,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 6, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 3 }, Ratio { p: 2, q: 1 }],
        length: 3,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 16, q: 1 }],
        length: 1,
    },
];
const FIXED_DIVIDER_PATTERNS: [DividerPattern; 17] = [
    DividerPattern {
        ratios: [Ratio { p: 8, q: 1 }, Ratio { p: 1, q: 8 }],
        length: 8,
    },
    DividerPattern {
        ratios: [Ratio { p: 6, q: 1 }, Ratio { p: 1, q: 6 }],
        length: 6,
    },
    DividerPattern {
        ratios: [Ratio { p: 4, q: 1 }, Ratio { p: 1, q: 4 }],
        length: 4,
    },
    DividerPattern {
        ratios: [Ratio { p: 3, q: 1 }, Ratio { p: 1, q: 3 }],
        length: 3,
    },
    DividerPattern {
        ratios: [Ratio { p: 2, q: 1 }, Ratio { p: 1, q: 2 }],
        length: 2,
    },
    DividerPattern {
        ratios: [Ratio { p: 3, q: 2 }, Ratio { p: 2, q: 3 }],
        length: 6,
    },
    DividerPattern {
        ratios: [Ratio { p: 4, q: 3 }, Ratio { p: 3, q: 4 }],
        length: 12,
    },
    DividerPattern {
        ratios: [Ratio { p: 5, q: 4 }, Ratio { p: 4, q: 5 }],
        length: 20,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 1 }, Ratio { p: 1, q: 1 }],
        length: 1,
    },
    DividerPattern {
        ratios: [Ratio { p: 4, q: 5 }, Ratio { p: 5, q: 4 }],
        length: 20,
    },
    DividerPattern {
        ratios: [Ratio { p: 3, q: 4 }, Ratio { p: 4, q: 3 }],
        length: 12,
    },
    DividerPattern {
        ratios: [Ratio { p: 2, q: 2 }, Ratio { p: 3, q: 2 }],
        length: 6,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 2 }, Ratio { p: 2, q: 1 }],
        length: 2,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 3 }, Ratio { p: 3, q: 1 }],
        length: 3,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 4 }, Ratio { p: 4, q: 1 }],
        length: 4,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 6 }, Ratio { p: 6, q: 1 }],
        length: 6,
    },
    DividerPattern {
        ratios: [Ratio { p: 1, q: 8 }, Ratio { p: 8, q: 1 }],
        length: 8,
    },
];
const INPUT_DIVIDER_RATIOS: [Ratio; 9] = [
    Ratio { p: 1, q: 4 },
    Ratio { p: 1, q: 3 },
    Ratio { p: 1, q: 2 },
    Ratio { p: 2, q: 3 },
    Ratio { p: 1, q: 1 },
    Ratio { p: 3, q: 2 },
    Ratio { p: 2, q: 1 },
    Ratio { p: 3, q: 1 },
    Ratio { p: 4, q: 1 },
];
const DRUM_PATTERNS: [[u8; 8]; 18] = [
    [1, 0, 0, 0, 2, 0, 0, 0],
    [0, 0, 1, 0, 2, 0, 0, 0],
    [1, 0, 1, 0, 2, 0, 0, 0],
    [0, 0, 1, 0, 2, 0, 0, 2],
    [1, 0, 1, 0, 2, 0, 1, 0],
    [0, 2, 1, 0, 2, 0, 0, 2],
    [1, 0, 0, 0, 2, 0, 1, 0],
    [0, 2, 1, 0, 2, 0, 1, 2],
    [1, 0, 0, 1, 2, 0, 0, 0],
    [0, 2, 1, 1, 2, 0, 1, 2],
    [1, 0, 0, 1, 2, 0, 1, 0],
    [0, 2, 1, 1, 2, 2, 1, 2],
    [1, 0, 0, 1, 2, 0, 1, 2],
    [0, 2, 0, 1, 2, 0, 1, 2],
    [1, 0, 1, 1, 2, 0, 1, 2],
    [2, 0, 1, 2, 0, 1, 2, 0],
    [1, 2, 1, 1, 2, 0, 1, 2],
    [2, 0, 1, 2, 0, 1, 2, 2],
];

#[derive(Clone, Debug)]
pub struct TGenerator {
    one_hertz: f32,
    model: u8,
    range: u8,
    rate: f32,
    bias: f32,
    jitter: f32,
    pulse_width_mean: f32,
    pulse_width_std: f32,
    master_phase: f32,
    jitter_multiplier: f32,
    phase_difference: f32,
    previous_external_ramp_value: f32,
    use_external_clock: bool,
    divider_pattern_length: i32,
    streak_counter: [i32; 16],
    markov_history: [i32; 16],
    markov_history_ptr: usize,
    drum_pattern_step: usize,
    drum_pattern_index: usize,
    sequence: RandomSequence,
    ramp_extractor: RampExtractor,
    // The original owns an initialized but unused RampGenerator.
    _ramp_generator: RampGenerator,
    slave_ramp: [SlaveRamp; 2],
    bias_quantizer: HysteresisQuantizer2,
    rate_quantizer: HysteresisQuantizer2,
}
impl TGenerator {
    pub fn new(stream: &mut RandomStream, sr: f32) -> Self {
        Self {
            one_hertz: 1.0 / sr,
            model: 0,
            range: 1,
            rate: 0.0,
            bias: 0.5,
            jitter: 0.0,
            pulse_width_mean: 0.0,
            pulse_width_std: 0.0,
            master_phase: 0.0,
            jitter_multiplier: 1.0,
            phase_difference: 0.0,
            previous_external_ramp_value: 0.0,
            use_external_clock: false,
            divider_pattern_length: 0,
            streak_counter: [0; 16],
            markov_history: [0; 16],
            markov_history_ptr: 0,
            drum_pattern_step: 0,
            drum_pattern_index: 0,
            sequence: RandomSequence::new(stream),
            ramp_extractor: RampExtractor::new(1000.0 / sr),
            _ramp_generator: RampGenerator::new(),
            slave_ramp: [SlaveRamp::new(), SlaveRamp::new()],
            bias_quantizer: HysteresisQuantizer2::new(17, 0.1, false),
            rate_quantizer: HysteresisQuantizer2::new(9, 0.05, false),
        }
    }
    pub fn set_model(&mut self, model: u8) {
        self.model = model;
    }
    pub fn set_range(&mut self, range: u8) {
        self.range = range;
    }
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
    }
    pub fn set_bias(&mut self, bias: f32) {
        self.bias = bias;
    }
    pub fn set_jitter(&mut self, jitter: f32) {
        self.jitter = jitter;
    }
    pub fn set_deja_vu(&mut self, value: f32) {
        self.sequence.set_deja_vu(value);
    }
    pub fn set_length(&mut self, length: i32) {
        self.sequence.set_length(length);
    }
    pub fn set_pulse_width_mean(&mut self, value: f32) {
        self.pulse_width_mean = value;
    }
    pub fn set_pulse_width_std(&mut self, value: f32) {
        self.pulse_width_std = value;
    }
    fn random_pulse_width(&self, u: f32) -> f32 {
        if self.pulse_width_std == 0.0 {
            0.05 + 0.9 * self.pulse_width_mean
        } else {
            0.05 + 0.9 * beta_distribution_sample(u, self.pulse_width_std, self.pulse_width_mean)
        }
    }
    fn complementary(&self, x: &[f32; 6]) -> i32 {
        let mut bitmask = 0;
        for i in 0..2 {
            if (x[2 + (i >> 1)] > self.bias) ^ (i & 1 != 0) {
                bitmask |= 1 << i;
            }
        }
        bitmask
    }
    fn independent(&self, x: &[f32; 6]) -> i32 {
        let mut bitmask = 0;
        for i in 0..2 {
            if (x[2 + i] > self.bias) ^ (i & 1 != 0) {
                bitmask |= 1 << i;
            }
        }
        bitmask
    }
    fn three_states(&self, x: &[f32; 6]) -> i32 {
        let mut bitmask = 0;
        let p_none = 0.75 - (self.bias - 0.5).abs();
        let threshold = p_none + (1.0 - p_none) * (0.25 + (self.bias * 0.5));
        for i in 0..2 {
            let u = x[2 + (i >> 1)];
            if u > p_none && ((u > threshold) ^ (i & 1 != 0)) {
                bitmask |= 1 << i;
            }
        }
        bitmask
    }
    fn drums(&mut self, x: &[f32; 6]) -> i32 {
        self.drum_pattern_step += 1;
        if self.drum_pattern_step >= 8 {
            self.drum_pattern_step = 0;
            let u = x[2] * 2.0 * (self.bias - 0.5).abs();
            self.drum_pattern_index = (18.0 * u) as i32 as usize;
            if self.bias <= 0.5 {
                self.drum_pattern_index -= self.drum_pattern_index % 2;
            }
        }
        // Native extension for the source's random endpoint u==1 producing18.
        DRUM_PATTERNS[self.drum_pattern_index.min(17)][self.drum_pattern_step] as i32
    }
    fn markov(&mut self, x: &[f32; 6]) -> i32 {
        let mut bitmask = 0;
        let b = 1.5 * self.bias - 0.5;
        let p = self.markov_history_ptr;
        self.markov_history[p] = 0;
        for i in 0..2 {
            let mask = 1 << i;
            let periodic = self.markov_history[(p + 8) % 16] & mask != 0;
            let simultaneous = self.markov_history[(p + 8) % 16] & !mask != 0;
            let dense = self.markov_history[(p + 1) % 16] & mask != 0;
            let alternate = self.markov_history[(p + 4) % 16] & !mask != 0;
            let mut logit = -1.5;
            logit += if self.streak_counter[i] > 24 {
                10.0
            } else {
                0.0
            };
            logit += 8.0 * b.abs() * if periodic { b } else { -b };
            logit -= 2.0 * if simultaneous { b } else { -b };
            logit -= 1.0 * if dense { b } else { 0.0 };
            logit += 1.0 * if alternate { b } else { 0.0 };
            logit = logit.clamp(-10.0, 10.0);
            let probability = LUT_LOGIT[(logit * 12.8 + 128.0) as i32 as usize];
            let mut state = x[2 + i] < probability;
            if self.sequence.deja_vu() >= x[4] {
                state = self.markov_history[(p + self.sequence.length() as usize) % 16] & mask != 0;
            }
            if state {
                bitmask |= mask;
                self.streak_counter[i] = 0;
            } else {
                self.streak_counter[i] = self.streak_counter[i].wrapping_add(1);
            }
        }
        self.markov_history[p] |= bitmask;
        self.markov_history_ptr = (p + 16 - 1) % 16;
        bitmask
    }
    fn schedule_output_pulses(&mut self, x: &[f32; 6], mut bitmask: i32) {
        for i in 0..2 {
            let width = self.random_pulse_width(x[i]);
            self.slave_ramp[i].init_bernoulli(bitmask & 1 != 0, width, 0.5);
            bitmask >>= 1;
        }
    }
    fn configure_slave_ramps(&mut self, x: &[f32; 6]) {
        match self.model {
            0 => {
                let mask = self.complementary(x);
                self.schedule_output_pulses(x, mask);
            }
            3 => {
                let mask = self.independent(x);
                self.schedule_output_pulses(x, mask);
            }
            5 => {
                let mask = self.three_states(x);
                self.schedule_output_pulses(x, mask);
            }
            2 => {
                let mask = self.drums(x);
                self.schedule_output_pulses(x, mask);
            }
            6 => {
                let mask = self.markov(x);
                self.schedule_output_pulses(x, mask);
            }
            1 | 4 => {
                self.divider_pattern_length -= 1;
                if self.divider_pattern_length <= 0 {
                    let mut pattern;
                    if self.model == 4 {
                        pattern =
                            FIXED_DIVIDER_PATTERNS[self.bias_quantizer.process(self.bias) as usize];
                    } else {
                        let strength = (self.bias - 0.5).abs() * 2.0;
                        let mut u = x[2];
                        u *= u + strength * strength * (1.0 - u);
                        u *= strength;
                        // Native extension for rounded random endpoint index17.
                        pattern = DIVIDER_PATTERNS[((u * 17.0) as usize).min(16)];
                        if self.bias < 0.5 {
                            pattern.ratios.swap(0, 1);
                        }
                    }
                    for i in 0..2 {
                        let width = self.random_pulse_width(x[i]);
                        self.slave_ramp[i].init_pattern(pattern.length, pattern.ratios[i], width);
                    }
                    self.divider_pattern_length = pattern.length;
                }
            }
            _ => {} // Caller validates source model range.
        }
    }
    pub fn process(
        &mut self,
        stream: &mut RandomStream,
        use_external_clock: bool,
        reset: &mut bool,
        external_clock: &[u8],
        ramps: &mut Ramps,
        gates: &mut [[bool; 2]],
    ) {
        let size = gates.len();
        assert!(size <= super::BLOCK && external_clock.len() >= size);
        let internal_frequency;
        if use_external_clock {
            if !self.use_external_clock {
                self.ramp_extractor.reset();
            }
            let mut ratio = INPUT_DIVIDER_RATIOS
                [self.rate_quantizer.process(1.05 * self.rate / 96.0 + 0.5) as usize];
            if self.range == 0 {
                ratio.q *= 4;
            } else if self.range == 2 {
                ratio.p *= 4;
            }
            while ratio.p % 2 == 0 && ratio.q % 2 == 0 {
                ratio.p /= 2;
                ratio.q /= 2;
            }
            self.ramp_extractor.process(
                ratio,
                true,
                reset,
                &external_clock[..size],
                &mut ramps.external[..size],
            );
            internal_frequency = 0.0;
        } else {
            let rate = if self.range == 2 {
                8.0
            } else if self.range == 0 {
                0.5
            } else {
                2.0
            };
            internal_frequency = rate * self.one_hertz * semitones_to_ratio(self.rate);
        }
        self.use_external_clock = use_external_clock;
        if *reset {
            for slave in &mut self.slave_ramp {
                slave.reset();
            }
            self.sequence.reset();
            self.divider_pattern_length = 0;
            self.drum_pattern_step = 8;
            if self.model != 4 {
                let mut vector = [0.0; 6];
                self.sequence.next_vector(&mut vector, stream);
                self.configure_slave_ramps(&vector);
            }
        }
        for i in 0..size {
            let mut frequency = if use_external_clock {
                ramps.external[i] - self.previous_external_ramp_value
            } else {
                internal_frequency
            };
            frequency += if frequency < 0.0 { 1.0 } else { 0.0 };
            let jittery_frequency = frequency * self.jitter_multiplier;
            self.master_phase += jittery_frequency;
            self.phase_difference += frequency - jittery_frequency;
            if self.master_phase > 1.0 {
                self.master_phase -= 1.0;
                let mut vector = [0.0; 6];
                self.sequence.next_vector(&mut vector, stream);
                let jitter_amount = self.jitter * self.jitter * self.jitter * self.jitter * 36.0;
                let x = fast_beta_distribution_sample(vector[5]);
                let mut multiplier = semitones_to_ratio((x * 2.0 - 1.0) * jitter_amount);
                multiplier *= if self.phase_difference > 0.0 {
                    1.0 + self.phase_difference
                } else {
                    1.0 / (1.0 - self.phase_difference)
                };
                self.jitter_multiplier = multiplier;
                self.configure_slave_ramps(&vector);
            }
            if internal_frequency != 0.0 {
                ramps.external[i] = self.master_phase;
            }
            self.previous_external_ramp_value = ramps.external[i];
            ramps.master[i] = self.master_phase;
            for j in 0..2 {
                let (phase, gate) = self.slave_ramp[j].process(frequency * self.jitter_multiplier);
                ramps.slave[j][i] = phase;
                gates[i][j] = gate;
            }
        }
    }
}
