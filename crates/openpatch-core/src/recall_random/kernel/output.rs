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
// Source: Marbles random/output_channel.{h,cc}, including the source's
// once-per-Process reacquisition (20 calls, 3.125ms at 32kHz / block 5).
use super::lag::LagProcessor;
use super::math::beta_distribution_sample;
use super::quantizer::{Quantizer, Scale};
use super::random::{RandomSequence, RandomStream};

#[derive(Clone, Copy)]
pub struct ScaleOffset {
    pub scale: f32,
    pub offset: f32,
}

#[derive(Clone)]
pub struct OutputChannel {
    pub spread: f32,
    pub bias: f32,
    pub steps: f32,
    pub scale_index: usize,
    pub register_mode: bool,
    pub register_value: f32,
    pub register_transposition: f32,
    pub scale_offset: ScaleOffset,
    previous_steps: f32,
    previous_phase: f32,
    reacquisition_counter: u32,
    previous_voltage: f32,
    voltage: f32,
    quantized_voltage: f32,
    lag_processor: LagProcessor,
    quantizer: [Quantizer; 6],
}
impl OutputChannel {
    pub fn new() -> Self {
        Self {
            spread: 0.5,
            bias: 0.5,
            steps: 0.5,
            scale_index: 0,
            register_mode: false,
            register_value: 0.0,
            register_transposition: 0.0,
            previous_steps: 0.0,
            previous_phase: 0.0,
            reacquisition_counter: 0,
            previous_voltage: 0.0,
            voltage: 0.0,
            quantized_voltage: 0.0,
            scale_offset: ScaleOffset {
                scale: 10.0,
                offset: -5.0,
            },
            lag_processor: LagProcessor::new(),
            quantizer: std::array::from_fn(|_| Quantizer::new(&Scale::root())),
        }
    }
    pub fn load_scale(&mut self, i: usize, scale: &Scale) {
        // Native guard: source requires an index in its six scale slots.
        if let Some(quantizer) = self.quantizer.get_mut(i) {
            quantizer.init(scale);
        }
    }
    pub fn quantize(&mut self, voltage: f32, amount: f32) -> f32 {
        // Native guard for invalid host indices; the valid source domain is 0..6.
        if let Some(quantizer) = self.quantizer.get_mut(self.scale_index) {
            quantizer.process(voltage, amount, false)
        } else {
            voltage
        }
    }
    fn generate_new_voltage(
        &mut self,
        random_sequence: &mut RandomSequence,
        stream: &mut RandomStream,
    ) -> f32 {
        let u = random_sequence.next_value(self.register_mode, self.register_value, stream);
        if self.register_mode {
            10.0 * (u - 0.5) + self.register_transposition
        } else {
            let degenerate_amount = (1.25 - self.spread * 25.0).clamp(0.0, 1.0);
            let bernoulli_amount = (self.spread * 25.0 - 23.75).clamp(0.0, 1.0);
            let mut value = beta_distribution_sample(u, self.spread, self.bias);
            let bernoulli_value = if u >= (1.0 - self.bias) {
                0.999999
            } else {
                0.0
            };
            value += degenerate_amount * (self.bias - value);
            value += bernoulli_amount * (bernoulli_value - value);
            value * self.scale_offset.scale + self.scale_offset.offset
        }
    }
    pub fn process(
        &mut self,
        random_sequence: &mut RandomSequence,
        stream: &mut RandomStream,
        phase: &[f32],
        output: &mut [[f32; 4]],
        channel: usize,
    ) {
        let size = output.len().min(phase.len());
        if size == 0 {
            return;
        }
        let mut steps = self.previous_steps;
        let increment = (self.steps - self.previous_steps) / size as f32;
        if self.reacquisition_counter != 0 {
            self.reacquisition_counter -= 1;
            let u = random_sequence.rewrite_value(self.register_value);
            self.voltage = 10.0 * (u - 0.5) + self.register_transposition;
            self.quantized_voltage = self.quantize(self.voltage, 2.0 * self.steps - 1.0);
        }
        for i in 0..size {
            steps += increment;
            if phase[i] < self.previous_phase {
                self.previous_voltage = self.voltage;
                self.voltage = self.generate_new_voltage(random_sequence, stream);
                self.lag_processor.reset_ramp();
                self.quantized_voltage = self.quantize(self.voltage, 2.0 * steps - 1.0);
                if self.register_mode {
                    self.reacquisition_counter = 20;
                }
            }
            output[i][channel] = if steps >= 0.5 {
                self.quantized_voltage
            } else {
                self.lag_processor
                    .process(self.voltage, 1.0 - 2.0 * steps, phase[i])
            };
            self.previous_phase = phase[i];
        }
        // ParameterInterpolator's destructor writes the accumulated float,
        // rather than the requested target, preserving block rounding.
        self.previous_steps = steps;
    }
}
