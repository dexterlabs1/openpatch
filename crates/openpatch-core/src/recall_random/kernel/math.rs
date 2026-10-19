// Copyright 2014-2015 Emilie Gillet. MIT license; see crate LICENSE-THIRD-PARTY.
// Source-derived stmlib dsp/units/hysteresis_quantizer and Marbles distributions.
use super::resources::*;

pub fn interpolate(table: &[f32], mut index: f32, scale: f32) -> f32 {
    index *= scale;
    let integral = index as i32;
    let fractional = index - integral as f32;
    // Native extension for source out-of-table inputs, including the terminal
    // endpoint without a C++ sentinel: repeat the nearest endpoint.
    let i = integral.max(0) as usize;
    let a = table[i.min(table.len() - 1)];
    let b = table[(i + 1).min(table.len() - 1)];
    a + (b - a) * fractional
}

pub fn semitones_to_ratio(semitones: f32) -> f32 {
    let pitch = semitones + 128.0;
    let integral = pitch as i32;
    let fractional = pitch - integral as f32;
    // Source accepts only table-domain pitch; native extension bounds indices.
    LUT_PITCH_RATIO_HIGH[integral.clamp(0, 256) as usize]
        * LUT_PITCH_RATIO_LOW[((fractional * 256.0) as i32).clamp(0, 256) as usize]
}

pub fn beta_distribution_sample(mut uniform: f32, mut spread: f32, mut bias: f32) -> f32 {
    let flip = bias > 0.5;
    if flip {
        uniform = 1.0 - uniform;
        bias = 1.0 - bias;
    }
    bias *= (5.0 - 1.0) * 2.0;
    spread *= 9.0 - 1.0;
    let bi = bias as i32;
    let bf = bias - bi as f32;
    let si = spread as i32;
    let sf = spread - si as f32;
    let cell = (bi * 10 + si) as usize;
    let mut offset = 0;
    if uniform <= 0.05 {
        offset = 129;
        uniform *= 20.0;
    } else if uniform >= 0.95 {
        offset = 258;
        uniform = (uniform - 0.95) * 20.0;
    }
    let x1y1 = interpolate(&DISTRIBUTIONS_TABLE[cell][offset..], uniform, 128.0);
    let x2y1 = interpolate(&DISTRIBUTIONS_TABLE[cell + 1][offset..], uniform, 128.0);
    let x1y2 = interpolate(&DISTRIBUTIONS_TABLE[cell + 10][offset..], uniform, 128.0);
    let x2y2 = interpolate(&DISTRIBUTIONS_TABLE[cell + 11][offset..], uniform, 128.0);
    let y1 = x1y1 + (x2y1 - x1y1) * sf;
    let y2 = x1y2 + (x2y2 - x1y2) * sf;
    let mut y = y1 + (y2 - y1) * bf;
    if flip {
        y = 1.0 - y;
    }
    y
}

pub fn fast_beta_distribution_sample(uniform: f32) -> f32 {
    interpolate(&DIST_ICDF_4_3, uniform, 128.0)
}

#[derive(Clone, Debug)]
pub struct HysteresisQuantizer2 {
    num_steps: i32,
    hysteresis: f32,
    scale: f32,
    offset: f32,
    quantized_value: i32,
}
impl HysteresisQuantizer2 {
    pub fn new(num_steps: i32, hysteresis: f32, symmetric: bool) -> Self {
        Self {
            num_steps,
            hysteresis,
            scale: if symmetric { num_steps - 1 } else { num_steps } as f32,
            offset: if symmetric { 0.0 } else { -0.5 },
            quantized_value: 0,
        }
    }
    pub fn process(&mut self, value: f32) -> i32 {
        self.process_with_base(0, value)
    }
    pub fn process_with_base(&mut self, base: i32, mut value: f32) -> i32 {
        value *= self.scale;
        value += self.offset;
        value += base as f32;
        let sign = if value > self.quantized_value as f32 {
            -1.0
        } else {
            1.0
        };
        let q = (value + sign * self.hysteresis + 0.5) as i32;
        self.quantized_value = q.clamp(0, self.num_steps - 1);
        self.quantized_value
    }
    pub fn num_steps(&self) -> i32 {
        self.num_steps
    }
    pub fn quantized_value(&self) -> i32 {
        self.quantized_value
    }
}
