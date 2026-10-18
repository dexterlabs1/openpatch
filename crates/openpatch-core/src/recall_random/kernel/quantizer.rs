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
// Source: Marbles random/quantizer.{h,cc}, discrete_distribution_quantizer.{h,cc},
// distributions.h, and stmlib/dsp/hysteresis_quantizer.h.

#[derive(Clone, Copy, Debug, Default)]
pub struct Degree {
    pub voltage: f32,
    pub weight: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub base_interval: f32,
    pub num_degrees: usize,
    pub degree: [Degree; 16],
}

impl Scale {
    pub fn root() -> Self {
        Self {
            base_interval: 1.0,
            num_degrees: 1,
            degree: [Degree::default(); 16],
        }
    }
    #[allow(clippy::excessive_precision)] // Keep the source decimal constant.
    pub fn major() -> Self {
        let weights = [255, 16, 128, 16, 192, 64, 8, 224, 16, 96, 32, 160];
        let mut scale = Self::root();
        scale.num_degrees = 12;
        for (i, weight) in weights.into_iter().enumerate() {
            scale.degree[i] = Degree {
                voltage: i as f32 * 0.0833333333,
                weight,
            };
        }
        scale
    }
    pub fn tenth() -> Self {
        let mut scale = Self::root();
        scale.num_degrees = 10;
        for i in 0..10 {
            scale.degree[i] = Degree {
                voltage: i as f32 * 0.1,
                weight: if i == 9 { 25 } else { 255 },
            };
        }
        scale
    }
    pub fn cell_voltage(&self, i: usize) -> f32 {
        let transposition = (i / self.num_degrees) as f32 * self.base_interval;
        self.degree[i % self.num_degrees].voltage + transposition
    }
}

#[derive(Clone, Copy, Default)]
struct Level {
    bitmask: u16,
    first: u8,
    last: u8,
}

#[derive(Clone)]
pub struct Quantizer {
    voltage: [f32; 16],
    level: [Level; 7],
    feedback: [f32; 7],
    base_interval: f32,
    base_interval_reciprocal: f32,
    num_degrees: usize,
    level_quantized_value: i32,
}

impl Quantizer {
    pub fn new(scale: &Scale) -> Self {
        let mut this = Self {
            voltage: [0.0; 16],
            level: [Level::default(); 7],
            feedback: [0.0; 7],
            base_interval: 1.0,
            base_interval_reciprocal: 1.0,
            num_degrees: 1,
            level_quantized_value: 0,
        };
        this.init(scale);
        this
    }
    pub fn init(&mut self, scale: &Scale) {
        let n = scale.num_degrees;
        if n == 0 || n > 16 || scale.base_interval == 0.0 {
            return;
        }
        self.num_degrees = n;
        self.base_interval = scale.base_interval;
        self.base_interval_reciprocal = 1.0 / scale.base_interval;
        let mut second_largest_threshold = 0;
        for i in 0..n {
            self.voltage[i] = scale.degree[i].voltage;
            if scale.degree[i].weight != 255 && scale.degree[i].weight >= second_largest_threshold {
                second_largest_threshold = scale.degree[i].weight;
            }
        }
        let mut thresholds = [0, 16, 32, 64, 128, 192, 255];
        if second_largest_threshold > 192 {
            thresholds[5] = second_largest_threshold;
        }
        for (t, threshold) in thresholds.into_iter().enumerate() {
            let mut l = Level {
                bitmask: 0,
                first: 0xff,
                last: 0,
            };
            for i in 0..n {
                if scale.degree[i].weight >= threshold {
                    l.bitmask |= 1 << i;
                    if l.first == 0xff {
                        l.first = i as u8;
                    }
                    l.last = i as u8;
                }
            }
            self.level[t] = l;
        }
        self.level_quantized_value = 0;
        self.feedback.fill(0.0);
    }
    pub fn process(&mut self, mut value: f32, amount: f32, hysteresis: bool) -> f32 {
        // Original HysteresisQuantizer (not HysteresisQuantizer2).
        let level_value = amount * 7.0;
        let feedback = if level_value > self.level_quantized_value as f32 {
            -0.25
        } else {
            0.25
        };
        let mut level = ((level_value + feedback + 0.5) as i32).clamp(0, 7);
        self.level_quantized_value = level;
        let mut quantized_voltage = value;
        if level > 0 {
            level -= 1;
            let level = level as usize;
            let raw_value = value;
            if hysteresis {
                value += self.feedback[level];
            }
            let note = value * self.base_interval_reciprocal;
            let mut note_integral = note as i32;
            let mut note_fractional = note - note_integral as f32;
            if value < 0.0 {
                note_integral -= 1;
                note_fractional += 1.0;
            }
            note_fractional *= self.base_interval;
            let l = self.level[level];
            // Native extension: an empty weighted level makes the C++ read
            // voltage_[255]. Preserve its defined domain and bypass that UB.
            if l.bitmask == 0 {
                return raw_value;
            }
            let mut a = self.voltage[l.last as usize] - self.base_interval;
            let mut b = self.voltage[l.first as usize] + self.base_interval;
            let mut bitmask = l.bitmask;
            for i in 0..self.num_degrees {
                if bitmask & 1 != 0 {
                    let v = self.voltage[i];
                    if note_fractional > v {
                        a = v;
                    } else {
                        b = v;
                        break;
                    }
                }
                bitmask >>= 1;
            }
            quantized_voltage = if note_fractional < (a + b) * 0.5 {
                a
            } else {
                b
            };
            quantized_voltage += note_integral as f32 * self.base_interval;
            self.feedback[level] = (quantized_voltage - raw_value) * 0.25;
        }
        quantized_voltage
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DistributionResult {
    pub token_id: usize,
    pub fraction: f32,
    pub start: f32,
    pub width: f32,
}

#[derive(Clone)]
pub struct DiscreteDistribution {
    sum: f32,
    cdf: [f32; 18],
    token_ids: [usize; 18],
    num_tokens: usize,
}

impl DiscreteDistribution {
    pub fn new() -> Self {
        Self {
            sum: 0.0,
            cdf: [0.0; 18],
            token_ids: [0; 18],
            num_tokens: 1,
        }
    }
    pub fn init(&mut self) {
        self.sum = 0.0;
        self.num_tokens = 1;
        self.cdf[0] = 0.0;
        self.token_ids[0] = 0;
    }
    pub fn add_token(&mut self, token_id: usize, weight: f32) {
        if weight <= 0.0 {
            return;
        }
        // At most 16 tokens, as in DiscreteDistribution<kMaxDegrees>.
        if self.num_tokens > 16 {
            return;
        }
        self.sum += weight;
        self.token_ids[self.num_tokens] = token_id;
        self.cdf[self.num_tokens] = self.sum;
        self.num_tokens += 1;
    }
    pub fn no_more_tokens(&mut self) {
        self.token_ids[self.num_tokens] = self.token_ids[self.num_tokens - 1];
        self.cdf[self.num_tokens] = self.sum + 1.0;
    }
    pub fn sample(&self, mut u: f32) -> DistributionResult {
        u *= self.sum;
        let mut n = 1;
        // std::upper_bound over [cdf_[1], cdf_[num_tokens_ + 1]).
        while n <= self.num_tokens && u >= self.cdf[n] {
            n += 1;
        }
        // Native extension for out-of-domain samples beyond the sentinel.
        n = n.min(self.num_tokens);
        let norm = 1.0 / self.sum;
        DistributionResult {
            token_id: self.token_ids[n],
            width: (self.cdf[n] - self.cdf[n - 1]) * norm,
            start: self.cdf[n - 1] * norm,
            fraction: (u - self.cdf[n - 1]) / (self.cdf[n] - self.cdf[n - 1]),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Cell {
    center: f32,
    width: f32,
    weight: f32,
}
impl Cell {
    fn scaled_width(self, amount: f32) -> f32 {
        let w = (8.0 * (self.weight - amount) + 0.5).clamp(0.0, 1.0);
        w * self.width
    }
}

#[derive(Clone)]
pub struct DiscreteDistributionQuantizer {
    base_interval: f32,
    base_interval_reciprocal: f32,
    num_cells: usize,
    cells: [Cell; 17],
    distribution: DiscreteDistribution,
}
impl DiscreteDistributionQuantizer {
    pub fn new(scale: &Scale) -> Self {
        let mut this = Self {
            base_interval: 1.0,
            base_interval_reciprocal: 1.0,
            num_cells: 2,
            cells: [Cell::default(); 17],
            distribution: DiscreteDistribution::new(),
        };
        this.init(scale);
        this
    }
    pub fn init(&mut self, scale: &Scale) {
        let n = scale.num_degrees;
        if n == 0 || n > 16 || scale.base_interval == 0.0 {
            return;
        }
        self.base_interval = scale.base_interval;
        self.base_interval_reciprocal = 1.0 / scale.base_interval;
        self.num_cells = n + 1;
        for i in 0..=n {
            let previous_voltage = scale.cell_voltage(if i == 0 { 0 } else { i - 1 });
            let next_voltage = scale.cell_voltage(if i == n { n } else { i + 1 });
            self.cells[i] = Cell {
                center: scale.cell_voltage(i),
                width: 0.5 * (next_voltage - previous_voltage),
                weight: scale.degree[i % n].weight as f32 / 256.0,
            };
        }
    }
    pub fn process(&mut self, value: f32, mut amount: f32) -> f32 {
        if amount < 0.0 {
            return value;
        }
        let note = value * self.base_interval_reciprocal;
        let mut note_integral = note as i32;
        let mut note_fractional = note - note_integral as f32;
        if value < 0.0 {
            note_integral -= 1;
            note_fractional += 1.0;
        }
        let scaled_amount = if amount < 0.25 {
            0.0
        } else {
            (amount - 0.25) * 1.333
        };
        self.distribution.init();
        for i in 0..self.num_cells - 1 {
            self.distribution
                .add_token(i, self.cells[i].scaled_width(scaled_amount));
        }
        self.distribution.no_more_tokens();
        let r = self.distribution.sample(note_fractional);
        let mut quantized_value = self.cells[r.token_id].center;
        let offset = note_integral as f32 * self.base_interval;
        quantized_value += offset;
        if amount < 0.25 {
            amount *= 4.0;
            let x = if r.token_id == 0 {
                r.fraction - 1.0
            } else if r.token_id == self.num_cells - 1 {
                -r.fraction
            } else {
                2.0 * ((r.fraction - 0.5).abs() - 0.5)
            };
            let slope = amount / (1.01 - amount);
            let y = (x * slope + 1.0).max(0.0);
            quantized_value -= y * (quantized_value - value);
        }
        quantized_value
    }
}
