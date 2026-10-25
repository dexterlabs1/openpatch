// Copyright 2014 Emilie Gillet.
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
// Rust adaptation (OpenPatch Grain Weather) of clouds/dsp/grain.h,
// granular_sample_player.h, window.h, correlator.h/.cc,
// wsola_sample_player.h and looping_sample_player.h at eurorack
// f4cfa7697095a28d292bff758beb51deb49b5a86.
//
// One deliberate difference: `Window::start` clears `done`, as every release
// of window.h did until the 2023 static-analysis edits (eurorack 0e3756f and
// fbb53ba, merged in d1d8839) each removed one of its two duplicated
// `done_ = false` assignments. Without it the stretch mode never schedules a
// window and is silent. See docs/GRAIN_WEATHER.md.

use super::buffer::AudioBuffer;
use super::memory::Memory;
use super::resources::{LUT_GRAIN_SIZE, LUT_SIN, LUT_WINDOW};
use super::stmlib::{
    crossfade, fast_rsqrt_carmack, interpolate, semitones_to_ratio, trunc_i32, Random,
};
use super::{Parameters, MAX_BLOCK_SIZE};
use std::cell::Cell;

pub(super) const MAX_NUM_GRAINS: usize = 64;
pub(super) const MAX_WSOLA_SIZE: i32 = 4096;
const CROSSFADE_DURATION: f32 = 64.0;

/// `std::min` for floats: returns `a` unless `b < a`.
#[inline]
fn min_f32(a: f32, b: f32) -> f32 {
    if b < a {
        b
    } else {
        a
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum GrainQuality {
    #[default]
    Low,
    Medium,
    High,
}

#[inline]
fn read(
    buffer: &AudioBuffer,
    memory: &Memory,
    quality: GrainQuality,
    integral: i32,
    fractional: u16,
) -> f32 {
    match quality {
        GrainQuality::Low => buffer.read_zoh(memory, integral, fractional),
        GrainQuality::Medium => buffer.read_linear(memory, integral, fractional),
        GrainQuality::High => buffer.read_hermite(memory, integral, fractional),
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Grain {
    first_sample: i32,
    phase: i32,
    phase_increment: i32,
    pre_delay: i32,
    envelope_smoothness: f32,
    envelope_slope: f32,
    envelope_phase: f32,
    envelope_phase_increment: f32,
    gain_l: f32,
    gain_r: f32,
    active: bool,
    recommended_quality: GrainQuality,
}

impl Grain {
    fn init(&mut self) {
        self.active = false;
        self.envelope_phase = 2.0;
    }

    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        pre_delay: i32,
        buffer_size: i32,
        start: i32,
        width: i32,
        phase_increment: i32,
        window_shape: f32,
        gain_l: f32,
        gain_r: f32,
        quality: GrainQuality,
    ) {
        self.pre_delay = pre_delay;
        self.first_sample = (start + buffer_size) % buffer_size;
        self.phase_increment = phase_increment;
        self.phase = 0;
        self.envelope_phase = 0.0;
        self.envelope_phase_increment = 2.0 / width as f32;
        if window_shape >= 0.5 {
            self.envelope_smoothness = (window_shape - 0.5) * 2.0;
            self.envelope_slope = 0.0;
        } else {
            self.envelope_smoothness = 0.0;
            self.envelope_slope = 0.5 / (window_shape + 0.01);
        }
        self.active = true;
        self.gain_l = gain_l;
        self.gain_r = gain_r;
        self.recommended_quality = quality;
    }

    fn render_envelope(
        &mut self,
        use_lut: bool,
        quality: GrainQuality,
        destination: &mut [f32],
        guard: &Cell<u64>,
    ) {
        let increment = self.envelope_phase_increment;
        let smoothness = self.envelope_smoothness;
        let slope = self.envelope_slope;
        let mut phase = self.envelope_phase;
        for slot in destination.iter_mut() {
            let mut gain = phase;
            gain = if gain >= 1.0 { 2.0 - gain } else { gain };
            if use_lut {
                if quality == GrainQuality::High {
                    let window = interpolate(&LUT_WINDOW, gain, 4096.0, guard);
                    gain += smoothness * (window - gain);
                }
            } else if quality >= GrainQuality::Medium {
                gain *= slope;
                if gain >= 1.0 {
                    gain = 1.0;
                }
            }
            phase += increment;
            if phase >= 2.0 {
                *slot = -1.0;
                break;
            }
            *slot = gain;
        }
        self.envelope_phase = phase;
    }

    #[allow(clippy::too_many_arguments)]
    fn overlap_add(
        &mut self,
        stereo: bool,
        buffers: &[AudioBuffer; 2],
        memory: &Memory,
        destination: &mut [[f32; 2]],
        envelope: &mut [f32; MAX_BLOCK_SIZE],
        guard: &Cell<u64>,
    ) {
        if !self.active {
            return;
        }
        // The pre-delay lets grains start at arbitrary samples in a block.
        let mut start = 0;
        while self.pre_delay != 0 && start < destination.len() {
            start += 1;
            self.pre_delay -= 1;
        }
        let destination = &mut destination[start..];
        let size = destination.len();
        let quality = self.recommended_quality;
        self.render_envelope(
            self.envelope_smoothness != 0.0,
            quality,
            &mut envelope[..size],
            guard,
        );

        let phase_increment = self.phase_increment;
        let first_sample = self.first_sample;
        let gain_l = self.gain_l;
        let gain_r = self.gain_r;
        let mut phase = self.phase;
        for (frame, &gain) in destination.iter_mut().zip(envelope.iter()) {
            let sample_index = first_sample.wrapping_add(phase >> 16);
            if gain == -1.0 {
                self.active = false;
                break;
            }
            let fractional = (phase & 65535) as u16;
            let l = read(&buffers[0], memory, quality, sample_index, fractional) * gain;
            if !stereo {
                frame[0] += l * gain_l;
                frame[1] += l * gain_r;
            } else {
                let r = read(&buffers[1], memory, quality, sample_index, fractional) * gain;
                frame[0] += l * gain_l + r * (1.0 - gain_r);
                frame[1] += r * gain_r + l * (1.0 - gain_l);
            }
            phase = phase.wrapping_add(phase_increment);
        }
        self.phase = phase;
    }
}

/// `clouds::GranularSamplePlayer`.
#[derive(Clone, Debug)]
pub(super) struct GranularSamplePlayer {
    max_num_grains: i32,
    num_midfi_grains: i32,
    num_channels: i32,
    num_grains: f32,
    gain_normalization: f32,
    grain_size_hint: f32,
    grain_rate_phasor: f32,
    grains: [Grain; MAX_NUM_GRAINS],
    available_grains: [i32; MAX_NUM_GRAINS],
    envelope_buffer: [f32; MAX_BLOCK_SIZE],
}

impl GranularSamplePlayer {
    pub(super) fn new() -> Self {
        Self {
            max_num_grains: 0,
            num_midfi_grains: 0,
            num_channels: 0,
            num_grains: 0.0,
            gain_normalization: 0.0,
            grain_size_hint: 0.0,
            grain_rate_phasor: 0.0,
            grains: [Grain::default(); MAX_NUM_GRAINS],
            available_grains: [0; MAX_NUM_GRAINS],
            envelope_buffer: [0.0; MAX_BLOCK_SIZE],
        }
    }

    pub(super) fn init(&mut self, num_channels: i32, max_num_grains: i32) {
        self.max_num_grains = max_num_grains;
        self.num_midfi_grains = 3 * max_num_grains / 4;
        self.gain_normalization = 1.0;
        for grain in self.grains.iter_mut() {
            grain.init();
        }
        self.num_grains = 0.0;
        self.num_channels = num_channels;
        self.grain_size_hint = 1024.0;
    }

    /// Grains currently sounding (for the panel's activity readout).
    pub(super) fn active_grains(&self) -> usize {
        self.grains[..self.max_num_grains.max(0) as usize]
            .iter()
            .filter(|grain| grain.active)
            .count()
    }

    fn fill_available_grains_list(&mut self) -> i32 {
        let mut count = 0;
        for i in 0..self.max_num_grains {
            if !self.grains[i as usize].active {
                self.available_grains[count as usize] = i;
                count += 1;
            }
        }
        count
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn play(
        &mut self,
        buffers: &[AudioBuffer; 2],
        memory: &Memory,
        parameters: &Parameters,
        random: &mut Random,
        out: &mut [[f32; 2]],
        guard: &Cell<u64>,
    ) {
        let size = out.len() as i32;
        let mut overlap = parameters.granular.overlap;
        overlap = overlap * overlap * overlap;
        let target_num_grains = self.max_num_grains as f32 * overlap;
        let mut p = target_num_grains / self.grain_size_hint;
        let space_between_grains = self.grain_size_hint / target_num_grains;
        if parameters.granular.use_deterministic_seed {
            p = -1.0;
        } else {
            self.grain_rate_phasor = -1000.0;
        }

        let mut num_available_grains = self.fill_available_grains_list();
        let mut seed_trigger = parameters.trigger;
        for t in 0..size {
            self.grain_rate_phasor += 1.0;
            let seed_probabilistic = random.get_float() < p && target_num_grains > self.num_grains;
            let seed_deterministic = self.grain_rate_phasor >= space_between_grains;
            let seed = seed_probabilistic || seed_deterministic || seed_trigger;
            if num_available_grains != 0 && seed {
                num_available_grains -= 1;
                let index = self.available_grains[num_available_grains as usize] as usize;
                let quality = if num_available_grains < self.num_midfi_grains {
                    GrainQuality::Medium
                } else {
                    GrainQuality::High
                };
                let buffer = &buffers[0];
                self.schedule_grain(
                    index,
                    parameters,
                    random,
                    t,
                    buffer.size(),
                    buffer.head() - size + t,
                    quality,
                    guard,
                );
                self.grain_rate_phasor = 0.0;
                seed_trigger = false;
            }
        }

        out.fill([0.0; 2]);
        let stereo = self.num_channels != 1;
        for i in 0..self.max_num_grains as usize {
            self.grains[i].overlap_add(
                stereo,
                buffers,
                memory,
                out,
                &mut self.envelope_buffer,
                guard,
            );
        }

        let active_grains = self.max_num_grains - num_available_grains;
        let error = active_grains as f32 - self.num_grains;
        self.num_grains += (if error > 0.0 { 0.9 } else { 0.2 }) * error;

        let mut gain_normalization = if self.num_grains > 2.0 {
            fast_rsqrt_carmack(self.num_grains - 1.0)
        } else {
            1.0
        };
        let window_gain = (1.0 + 2.0 * parameters.granular.window_shape).clamp(1.0, 2.0);
        gain_normalization *= crossfade(1.0, window_gain, parameters.granular.overlap);
        for frame in out.iter_mut() {
            self.gain_normalization += 0.01 * (gain_normalization - self.gain_normalization);
            frame[0] *= self.gain_normalization;
            frame[1] *= self.gain_normalization;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn schedule_grain(
        &mut self,
        index: usize,
        parameters: &Parameters,
        random: &mut Random,
        pre_delay: i32,
        buffer_size: i32,
        buffer_head: i32,
        quality: GrainQuality,
        guard: &Cell<u64>,
    ) {
        let position = parameters.position;
        let pitch = parameters.pitch;
        let window_shape = parameters.granular.window_shape;
        let mut grain_size = interpolate(&LUT_GRAIN_SIZE, parameters.size, 256.0, guard);
        let pitch_ratio = semitones_to_ratio(pitch);
        let inv_pitch_ratio = semitones_to_ratio(-pitch);
        let pan = 0.5 + parameters.stereo_spread * (random.get_float() - 0.5);
        let (gain_l, gain_r) = if self.num_channels == 1 {
            (
                interpolate(&LUT_SIN, pan, 256.0, guard),
                interpolate(&LUT_SIN[256..], pan, 256.0, guard),
            )
        } else if pan < 0.5 {
            (1.0, 2.0 * pan)
        } else {
            (2.0 * (1.0 - pan), 1.0)
        };

        if pitch_ratio > 1.0 {
            // The grain's play-head moves faster than the record-head; make
            // sure the grain does not consume too much data.
            grain_size = min_f32(grain_size, buffer_size as f32 * 0.25 * inv_pitch_ratio);
        }

        let eaten_by_play_head = grain_size * pitch_ratio;
        let eaten_by_recording_head = grain_size;
        let mut available = 0.0f32;
        available += buffer_size as f32;
        available -= eaten_by_play_head;
        available -= eaten_by_recording_head;

        let size = trunc_i32(grain_size) & !1;
        let start = buffer_head - trunc_i32(position * available + eaten_by_play_head);
        self.grains[index].start(
            pre_delay,
            buffer_size,
            start,
            size,
            (pitch_ratio * 65536.0) as u32 as i32,
            window_shape,
            gain_l,
            gain_r,
            quality,
        );
        self.grain_size_hint += 0.1 * (grain_size - self.grain_size_hint);
    }
}

/// `clouds::Window`, the WSOLA grain.
#[derive(Clone, Copy, Debug, Default)]
struct Window {
    first_sample: i32,
    phase: i32,
    phase_increment: i32,
    envelope_phase_increment: f32,
    done: bool,
    half: bool,
    regenerated: bool,
}

impl Window {
    fn init(&mut self) {
        self.done = true;
        self.regenerated = false;
        self.half = false;
    }

    fn start(&mut self, buffer_size: i32, start: i32, width: i32, phase_increment: i32) {
        self.first_sample = (start + buffer_size) % buffer_size;
        self.phase_increment = phase_increment;
        self.phase = 0;
        // Restored from window.h before eurorack 0e3756f/fbb53ba; see above.
        self.done = false;
        self.regenerated = false;
        self.envelope_phase_increment = 2.0 / width as f32;
    }

    fn overlap_add(
        &mut self,
        buffers: &[AudioBuffer; 2],
        memory: &Memory,
        sample: &mut [f32; 2],
        channels: i32,
    ) {
        if self.done {
            return;
        }
        let phase_integral = self.phase >> 16;
        let phase_fractional = (self.phase & 0xffff) as u16;
        let sample_index = self.first_sample.wrapping_add(phase_integral);
        let envelope_phase = phase_integral as f32 * self.envelope_phase_increment;
        self.done = envelope_phase >= 2.0;
        self.half = envelope_phase >= 1.0;
        let gain = if envelope_phase >= 1.0 {
            2.0 - envelope_phase
        } else {
            envelope_phase
        };
        let l = buffers[0].read_hermite(memory, sample_index, phase_fractional) * gain;
        if channels == 1 {
            sample[0] += l;
            sample[1] += l;
        } else if channels == 2 {
            let r = buffers[1].read_hermite(memory, sample_index, phase_fractional) * gain;
            sample[0] += l;
            sample[1] += r;
        }
        self.phase = self.phase.wrapping_add(self.phase_increment);
    }

    fn needs_regeneration(&self) -> bool {
        self.half && !self.regenerated
    }
}

/// `clouds::Correlator`: 1-bit cross-correlation search over sign words held
/// in the memory image.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Correlator {
    source: usize,
    destination: usize,
    offset: i32,
    increment: i32,
    size: i32,
    candidate: i32,
    best_score: u32,
    best_match: i32,
    done: bool,
}

impl Correlator {
    pub(super) fn init(&mut self, source: usize, destination: usize) {
        self.source = source;
        self.destination = destination;
        self.offset = 0;
        self.best_match = 0;
        self.done = true;
    }

    fn start_search(&mut self, size: i32, offset: i32, increment: i32) {
        self.offset = offset;
        self.increment = increment;
        self.best_score = 0;
        self.best_match = 0;
        self.candidate = 0;
        self.size = size;
        self.done = false;
    }

    fn best_match(&self) -> i32 {
        self.offset
            .wrapping_add(self.best_match.wrapping_mul(self.increment >> 4) >> 12)
    }

    pub(super) fn evaluate_some_candidates(&mut self, memory: &Memory) {
        let mut num_candidates = ((self.size >> 2) + 16) as u32 as usize;
        while num_candidates != 0 {
            self.evaluate_next_candidate(memory);
            num_candidates -= 1;
        }
    }

    fn evaluate_next_candidate(&mut self, memory: &Memory) {
        if self.done {
            return;
        }
        let num_words = (self.size as u32) >> 5;
        let offset_words = (self.candidate as u32 >> 5) as usize;
        let offset_bits = self.candidate as u32 & 0x1f;
        let destination = self.destination + 4 * offset_words;
        let mut xcorr = 0u32;
        for i in 0..num_words as usize {
            let source_bits = memory.u32(self.source + 4 * i);
            let mut destination_bits = memory.u32(destination + 4 * i) << offset_bits;
            // `x >> (32 - offset_bits)`: a 32-bit shift at offset zero is
            // undefined in C++. Cortex-M yields zero for it, and the pinned
            // oracle's clang arm64 code computes (x >> 1) >> (31 - offset),
            // also zero (verified by disassembly), so the term vanishes.
            if offset_bits != 0 {
                destination_bits |= memory.u32(destination + 4 * (i + 1)) >> (32 - offset_bits);
            }
            xcorr += (!(source_bits ^ destination_bits)).count_ones();
        }
        if xcorr > self.best_score {
            self.best_match = self.candidate;
            self.best_score = xcorr;
        }
        self.candidate += 1;
        self.done = self.candidate >= self.size;
    }
}

/// `clouds::WSOLASamplePlayer`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct WsolaSamplePlayer {
    windows: [Window; 2],
    window_size: i32,
    num_channels: i32,
    pitch: f32,
    smoothed_pitch: f32,
    position: f32,
    size_factor: f32,
    next_pitch_ratio: f32,
    correlator_loaded: bool,
    search_source: i32,
    search_target: i32,
    env_phase: f32,
    env_phase_increment: f32,
    elapsed: i32,
}

impl WsolaSamplePlayer {
    pub(super) fn init(&mut self, num_channels: i32) {
        self.num_channels = num_channels;
        self.pitch = 0.0;
        self.position = 0.0;
        self.smoothed_pitch = 0.0;
        self.windows[0].init();
        self.windows[1].init();
        self.next_pitch_ratio = 1.0;
        self.correlator_loaded = true;
        self.search_source = 0;
        self.search_target = 0;
        self.window_size = MAX_WSOLA_SIZE / 2;
        self.env_phase = 0.0;
        self.env_phase_increment = 0.5;
        self.elapsed = 0;
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn play(
        &mut self,
        buffers: &[AudioBuffer; 2],
        memory: &Memory,
        correlator: &mut Correlator,
        parameters: &Parameters,
        out: &mut [[f32; 2]],
    ) {
        self.elapsed += 1;
        if parameters.trigger {
            self.env_phase = 0.0;
            self.env_phase_increment = (1.0 / self.elapsed as f32).clamp(0.0001, 0.1);
            self.elapsed = 0;
        }
        self.env_phase += self.env_phase_increment;
        if self.env_phase >= 1.0 {
            self.env_phase = 1.0;
        }
        self.position = parameters.position;
        self.position += (1.0 - self.env_phase) * (1.0 - self.position);
        self.pitch = parameters.pitch;
        self.size_factor = parameters.size;

        if self.windows[0].done && self.windows[1].done {
            self.windows[1].regenerated = true;
            self.schedule_aligned_window(buffers, correlator, 0);
        }

        for frame in out.iter_mut() {
            *frame = [0.0; 2];
            for i in 0..2 {
                self.windows[i].overlap_add(buffers, memory, frame, self.num_channels);
            }
            for i in 0..2 {
                if self.windows[i].needs_regeneration() {
                    self.windows[i].regenerated = true;
                    self.schedule_aligned_window(buffers, correlator, 1 - i);
                    self.windows[1 - i].overlap_add(buffers, memory, frame, self.num_channels);
                }
            }
        }
    }

    fn read_sign_bits(
        buffers: &[AudioBuffer; 2],
        memory: &mut Memory,
        stereo: bool,
        phase_increment: i32,
        mut source: i32,
        size: i32,
        destination: usize,
    ) -> i32 {
        let mut phase: i32 = 0;
        let mut bits: u32 = 0;
        let mut bit_counter: u32 = 0;
        let mut num_samples = 0;
        if source < 0 {
            source += buffers[0].size();
        }
        while (phase >> 16) < size {
            let integral = source + (phase >> 16);
            let fractional = (phase & 0xffff) as u16;
            let mut s = buffers[0].read_linear(memory, integral, fractional);
            if stereo {
                s += buffers[1].read_linear(memory, integral, fractional);
            }
            bits |= if s > 0.0 { 1 } else { 0 };
            if bit_counter & 0x1f == 0x1f {
                memory.set_u32(destination + 4 * (bit_counter >> 5) as usize, bits);
                num_samples += 32;
            }
            bit_counter += 1;
            bits <<= 1;
            phase = phase.wrapping_add(phase_increment);
        }
        while bit_counter & 0x1f != 0 {
            if bit_counter & 0x1f == 0x1f {
                memory.set_u32(destination + 4 * (bit_counter >> 5) as usize, bits);
                num_samples += 32;
            }
            bit_counter += 1;
            bits <<= 1;
        }
        num_samples
    }

    pub(super) fn load_correlator(
        &mut self,
        buffers: &[AudioBuffer; 2],
        memory: &mut Memory,
        correlator: &mut Correlator,
    ) {
        if self.correlator_loaded {
            return;
        }
        let mut stride = (self.window_size as f32 / 2048.0).clamp(1.0, 2.0);
        stride *= 65536.0;
        let ratio = if self.next_pitch_ratio < 1.25 {
            1.25
        } else {
            self.next_pitch_ratio
        };
        let increment = trunc_i32(stride * ratio);
        let stereo = self.num_channels != 1;
        let num_samples = Self::read_sign_bits(
            buffers,
            memory,
            stereo,
            increment,
            self.search_source,
            self.window_size,
            correlator.source,
        );
        Self::read_sign_bits(
            buffers,
            memory,
            stereo,
            increment,
            self.search_target - self.window_size,
            self.window_size * 2,
            correlator.destination,
        );
        correlator.start_search(
            num_samples,
            self.search_target - self.window_size + (self.window_size >> 1),
            increment,
        );
        self.correlator_loaded = true;
    }

    fn schedule_aligned_window(
        &mut self,
        buffers: &[AudioBuffer; 2],
        correlator: &Correlator,
        window: usize,
    ) {
        let buffer = &buffers[0];
        let next_window_position = correlator.best_match();
        self.correlator_loaded = false;
        self.windows[window].start(
            buffer.size(),
            next_window_position - (self.window_size >> 1),
            self.window_size,
            (self.next_pitch_ratio * 65536.0) as u32 as i32,
        );

        let mut pitch_error = self.pitch - self.smoothed_pitch;
        let pitch_error_sign = if pitch_error < 0.0 { -1.0 } else { 1.0 };
        pitch_error *= pitch_error_sign;
        if pitch_error >= 12.0 {
            pitch_error = 12.0;
        }
        self.smoothed_pitch += pitch_error * pitch_error_sign;
        let pitch_ratio = semitones_to_ratio(self.smoothed_pitch);
        let inv_pitch_ratio = semitones_to_ratio(-self.smoothed_pitch);
        self.next_pitch_ratio = pitch_ratio;

        let size_factor = semitones_to_ratio((self.size_factor - 1.0) * 60.0);
        let mut new_window_size = trunc_i32(size_factor * MAX_WSOLA_SIZE as f32);
        if (new_window_size - self.window_size).abs() > 64 {
            let error = (new_window_size - self.window_size) >> 5;
            new_window_size = self.window_size + error;
            self.window_size = new_window_size - (new_window_size % 4);
        }

        // The center offset of the window we want to mix in.
        let mut limit = buffer.size();
        limit -= trunc_i32(2.0 * self.window_size as f32 * inv_pitch_ratio);
        limit -= 2 * self.window_size;
        if limit < 0 {
            limit = 0;
        }
        let position = self.position;
        let mut target_position = buffer.head();
        target_position -= trunc_i32(limit as f32 * position);
        target_position -= self.window_size;
        self.search_source = next_window_position;
        self.search_target = target_position;
    }
}

/// `clouds::LoopingSamplePlayer`.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LoopingSamplePlayer {
    phase: f32,
    current_delay: f32,
    loop_point: f32,
    loop_duration: f32,
    tail_start: f32,
    tail_duration: f32,
    loop_reset: f32,
    synchronized: bool,
    num_channels: i32,
    tap_delay: i32,
    tap_delay_counter: i32,
}

#[inline]
fn hermite_at(buffer: &AudioBuffer, memory: &Memory, position: i32) -> f32 {
    buffer.read_hermite(
        memory,
        position >> 12,
        (position as u32).wrapping_shl(4) as u16,
    )
}

impl LoopingSamplePlayer {
    pub(super) fn init(&mut self, num_channels: i32) {
        self.num_channels = num_channels;
        self.phase = 0.0;
        self.current_delay = 0.0;
        self.loop_point = 0.0;
        self.loop_duration = 0.0;
        self.tap_delay = 0;
        self.tap_delay_counter = 0;
        self.synchronized = false;
        self.tail_duration = 1.0;
    }

    pub(super) fn synchronized(&self) -> bool {
        self.synchronized
    }

    pub(super) fn play(
        &mut self,
        buffers: &[AudioBuffer; 2],
        memory: &Memory,
        parameters: &Parameters,
        out: &mut [[f32; 2]],
    ) {
        let buffer = &buffers[0];
        let size = out.len() as i32;
        let max_delay = trunc_i32(buffer.size() as f32 - CROSSFADE_DURATION);
        self.tap_delay_counter += size;
        if self.tap_delay_counter > max_delay {
            self.tap_delay = 0;
            self.tap_delay_counter = 0;
            self.synchronized = false;
        }
        if parameters.trigger {
            self.tap_delay = self.tap_delay_counter;
            self.tap_delay_counter = 0;
            self.synchronized = self.tap_delay > 128;
            self.loop_reset = self.phase;
            self.phase = 0.0;
        }

        if !parameters.freeze {
            for (k, frame) in out.iter_mut().enumerate() {
                // The source reads `size` after its post-decrement.
                let remaining = size - 1 - k as i32;
                let mut target_delay = parameters.position * max_delay as f32;
                if self.synchronized {
                    target_delay = self.tap_delay as f32;
                }
                let error = target_delay - self.current_delay;
                let delay = self.current_delay + 0.00005 * error;
                self.current_delay = delay;
                let mut delay_int = (buffer.head() - 4 - remaining + buffer.size()) << 12;
                delay_int -= trunc_i32(delay * 4096.0);
                let l = hermite_at(buffer, memory, delay_int);
                if self.num_channels == 1 {
                    *frame = [l, l];
                } else {
                    let r = hermite_at(&buffers[1], memory, delay_int);
                    *frame = [l, r];
                }
            }
            self.phase = 0.0;
        } else {
            let mut loop_point = parameters.position * max_delay as f32 * 15.0 / 16.0;
            loop_point += CROSSFADE_DURATION;
            let d = parameters.size;
            let mut loop_duration = (0.01 + 0.99 * d * d * d) * max_delay as f32;
            if self.synchronized {
                loop_duration = self.tap_delay as f32;
            }
            if loop_point + loop_duration >= max_delay as f32 {
                loop_point = max_delay as f32 - loop_duration;
            }
            let phase_increment = if self.synchronized {
                1.0
            } else {
                semitones_to_ratio(parameters.pitch)
            };

            for frame in out.iter_mut() {
                if self.phase >= self.loop_duration || self.phase == 0.0 {
                    if self.phase >= self.loop_duration {
                        self.loop_reset = self.loop_duration;
                    }
                    if self.loop_reset >= self.loop_duration {
                        self.loop_reset = self.loop_duration;
                    }
                    self.tail_start = self.loop_duration - self.loop_reset + self.loop_point;
                    self.phase = 0.0;
                    self.tail_duration =
                        min_f32(CROSSFADE_DURATION, CROSSFADE_DURATION * phase_increment);
                    self.loop_point = loop_point;
                    self.loop_duration = loop_duration;
                }
                self.phase += phase_increment;

                let mut gain = 1.0f32;
                if self.tail_duration != 0.0 {
                    gain = (self.phase / self.tail_duration).clamp(0.0, 1.0);
                }
                let delay_int = (buffer.head() - 4 + buffer.size()) << 12;
                let position = delay_int
                    - trunc_i32((self.loop_duration - self.phase + self.loop_point) * 4096.0);
                let l = hermite_at(buffer, memory, position);
                if self.num_channels == 1 {
                    *frame = [l * gain, l * gain];
                } else {
                    let r = hermite_at(&buffers[1], memory, position);
                    *frame = [l * gain, r * gain];
                }

                if gain != 1.0 {
                    let gain = 1.0 - gain;
                    let position = delay_int - trunc_i32((-self.phase + self.tail_start) * 4096.0);
                    let l = hermite_at(buffer, memory, position);
                    if self.num_channels == 1 {
                        frame[0] += l * gain;
                        frame[1] += l * gain;
                    } else {
                        let r = hermite_at(&buffers[1], memory, position);
                        frame[0] += l * gain;
                        frame[1] += r * gain;
                    }
                }
            }
        }
    }
}
