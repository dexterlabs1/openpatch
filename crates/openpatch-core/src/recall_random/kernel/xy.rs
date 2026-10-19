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
// Source: Marbles random/x_y_generator.{h,cc}.
use super::output::{OutputChannel, ScaleOffset};
use super::quantizer::Scale;
use super::ramp::{RampDivider, RampExtractor};
use super::random::{RandomSequence, RandomStream};
use super::{Ramps, Ratio, BLOCK};

pub const CLOCK_SOURCE_INTERNAL_T1_T2_T3: u8 = 0;
pub const CLOCK_SOURCE_INTERNAL_T1: u8 = 1;
pub const CLOCK_SOURCE_INTERNAL_T2: u8 = 2;
pub const CLOCK_SOURCE_INTERNAL_T3: u8 = 3;
pub const CLOCK_SOURCE_EXTERNAL: u8 = 4;
pub const CONTROL_MODE_IDENTICAL: u8 = 0;
pub const CONTROL_MODE_BUMP: u8 = 1;
pub const CONTROL_MODE_TILT: u8 = 2;
pub const VOLTAGE_RANGE_NARROW: u8 = 0;
pub const VOLTAGE_RANGE_POSITIVE: u8 = 1;
pub const VOLTAGE_RANGE_FULL: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct GroupSettings {
    pub control_mode: u8,
    pub voltage_range: u8,
    pub register_mode: bool,
    pub register_value: f32,
    pub spread: f32,
    pub bias: f32,
    pub steps: f32,
    pub deja_vu: f32,
    pub scale_index: usize,
    pub length: i32,
    pub ratio: Ratio,
}

#[derive(Clone)]
pub struct XYGenerator {
    random_sequence: [RandomSequence; 4],
    output_channel: [OutputChannel; 4],
    ramp_extractor: RampExtractor,
    ramp_divider: RampDivider,
    external_clock_stabilization_counter: i32,
    use_shifted_sequences: [bool; 4],
}
impl XYGenerator {
    pub fn new(stream: &mut RandomStream, sr: f32) -> Self {
        Self {
            random_sequence: std::array::from_fn(|_| RandomSequence::new(stream)),
            output_channel: std::array::from_fn(|_| OutputChannel::new()),
            ramp_extractor: RampExtractor::new(8000.0 / sr),
            ramp_divider: RampDivider::new(),
            external_clock_stabilization_counter: 16,
            use_shifted_sequences: [false; 4],
        }
    }
    pub fn load_scale(&mut self, scale_index: usize, scale: &Scale) {
        // Source overload loads X only. Y has a separate channel-scale API.
        for channel in &mut self.output_channel[..3] {
            channel.load_scale(scale_index, scale);
        }
    }
    pub fn load_channel_scale(&mut self, channel: usize, scale_index: usize, scale: &Scale) {
        // Native guard: source requires channel 0..4 and scale_index 0..6.
        if let Some(channel) = self.output_channel.get_mut(channel) {
            channel.load_scale(scale_index, scale);
        }
    }
    /// Native host-only adoption: copy an already prepared slot to X and Y.
    pub(crate) fn copy_prepared_scale(&mut self, slot: usize, prepared: &Self) {
        for (channel, source) in self.output_channel.iter_mut().zip(&prepared.output_channel) {
            channel.copy_prepared_scale(slot, source);
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        stream: &mut RandomStream,
        clock_source: u8,
        x_settings: &GroupSettings,
        y_settings: &GroupSettings,
        reset: &mut bool,
        external_clock: &[u8],
        ramps: &mut Ramps,
        output: &mut [[f32; 4]],
    ) {
        let size = output.len().min(BLOCK).min(external_clock.len());
        if size == 0 {
            return;
        }
        if clock_source != CLOCK_SOURCE_EXTERNAL {
            self.external_clock_stabilization_counter = 16;
        } else if self.external_clock_stabilization_counter != 0 {
            self.external_clock_stabilization_counter -= 1;
            if self.external_clock_stabilization_counter == 0 {
                self.ramp_extractor.reset();
            }
        }
        if clock_source == CLOCK_SOURCE_EXTERNAL {
            self.ramp_extractor.process(
                Ratio { p: 1, q: 1 },
                false,
                reset,
                &external_clock[..size],
                &mut ramps.slave[0][..size],
            );
            if self.external_clock_stabilization_counter != 0 {
                ramps.slave[0][..size].fill(0.0);
            }
        }
        // Fixed stack copies replace source ramp pointers, preserving all writes
        // to the caller's slave[0]/external arrays and channel ordering.
        let channel_ramp = match clock_source {
            CLOCK_SOURCE_EXTERNAL | CLOCK_SOURCE_INTERNAL_T1 => [ramps.slave[0]; 3],
            CLOCK_SOURCE_INTERNAL_T2 => [ramps.master; 3],
            CLOCK_SOURCE_INTERNAL_T3 => [ramps.slave[1]; 3],
            _ => [ramps.slave[0], ramps.master, ramps.slave[1]],
        };
        if *reset {
            self.ramp_divider.reset();
        }
        self.ramp_divider.process(
            y_settings.ratio,
            &channel_ramp[1][..size],
            &mut ramps.external[..size],
        );
        for i in 0..4 {
            let channel = &mut self.output_channel[i];
            let settings = if i < 3 { x_settings } else { y_settings };
            match settings.voltage_range {
                VOLTAGE_RANGE_NARROW => {
                    channel.scale_offset = ScaleOffset {
                        scale: 2.0,
                        offset: 0.0,
                    }
                }
                VOLTAGE_RANGE_POSITIVE => {
                    channel.scale_offset = ScaleOffset {
                        scale: 5.0,
                        offset: 0.0,
                    }
                }
                VOLTAGE_RANGE_FULL => {
                    channel.scale_offset = ScaleOffset {
                        scale: 10.0,
                        offset: -5.0,
                    }
                }
                _ => {}
            }
            let mut amount = 1.0;
            if settings.control_mode == CONTROL_MODE_BUMP {
                amount = if i == 1 { 1.0 } else { -1.0 };
            } else if settings.control_mode == CONTROL_MODE_TILT {
                amount = 2.0 * i as f32 / 2.0 - 1.0;
            }
            channel.spread = 0.5 + (settings.spread - 0.5) * amount;
            channel.bias = 0.5 + (settings.bias - 0.5) * amount;
            channel.steps =
                0.5 + (settings.steps - 0.5) * if settings.register_mode { 1.0 } else { amount };
            channel.scale_index = settings.scale_index;
            channel.register_mode = settings.register_mode;
            channel.register_value = settings.register_value;
            channel.register_transposition = 4.0 * settings.spread * (settings.bias - 0.5) * amount;
            let own_sequence = &mut self.random_sequence[i];
            own_sequence.record();
            own_sequence.set_length(settings.length);
            own_sequence.set_deja_vu(settings.deja_vu);
            if *reset {
                own_sequence.reset();
            }
            let mut sequence_index = i;
            let mut use_shifted_sequences = false;
            if clock_source != CLOCK_SOURCE_INTERNAL_T1_T2_T3 && i > 0 && i < 3 {
                sequence_index = 0;
                if settings.register_mode {
                    use_shifted_sequences = true;
                    let shift = if settings.control_mode == CONTROL_MODE_IDENTICAL {
                        i as u32
                    } else if settings.control_mode == CONTROL_MODE_BUMP {
                        if i == 2 {
                            1
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    self.random_sequence[0].replay_shifted(shift);
                } else {
                    const HASHES: [u32; 3] = [0, 0xbeca55e5, 0xf0cacc1a];
                    self.random_sequence[0].replay_pseudo_random(HASHES[i]);
                }
            }
            if !use_shifted_sequences && self.use_shifted_sequences[i] && sequence_index != 0 {
                // Index-based redo state in RandomSequence gives source Clone's
                // rebased loop/history pointers, including mode transitions.
                self.random_sequence[sequence_index] = self.random_sequence[0].clone();
            }
            self.use_shifted_sequences[i] = use_shifted_sequences;
            let phase = if i < 3 {
                &channel_ramp[i][..size]
            } else {
                &ramps.external[..size]
            };
            channel.process(
                &mut self.random_sequence[sequence_index],
                stream,
                phase,
                &mut output[..size],
                i,
            );
        }
    }
}
