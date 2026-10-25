// Copyright 2014-2015 Emilie Gillet.
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
// Rust adaptation (OpenPatch Grain Weather) of the Mutable Instruments Clouds
// granular processor: clouds/dsp/granular_processor.h/.cc, parameters.h and
// frame.h at eurorack f4cfa7697095a28d292bff758beb51deb49b5a86, with the
// players, effects and phase vocoder in the submodules. The processor runs at
// the source's native 32 kHz in 32-frame blocks of 16-bit codec frames.
// `prepare` is the firmware main-loop task; the host calls it after every
// block (see docs/GRAIN_WEATHER.md for the exact cadence).

#[path = "kernel/buffer.rs"]
mod buffer;
#[path = "kernel/fx.rs"]
mod fx;
#[path = "kernel/memory.rs"]
mod memory;
#[path = "kernel/players.rs"]
mod players;
#[path = "kernel/pvoc.rs"]
mod pvoc;
#[path = "kernel/resources.rs"]
mod resources;
#[path = "kernel/stmlib.rs"]
mod stmlib;

use buffer::AudioBuffer;
use fx::{Diffuser, PitchShifter, Reverb, SampleRateConverter};
use memory::{Allocator, Memory, LARGE_BYTES, SMALL_BASE, SMALL_BYTES};
use players::{
    Correlator, GranularSamplePlayer, LoopingSamplePlayer, WsolaSamplePlayer, MAX_WSOLA_SIZE,
};
use pvoc::PhaseVocoder;
use resources::{LUT_XFADE_IN, LUT_XFADE_OUT};
use std::cell::Cell;
use stmlib::{interpolate, semitones_to_ratio, soft_convert, soft_limit, FilterMode, Random, Svf};

pub(crate) const MAX_BLOCK_SIZE: usize = 32;
pub(crate) const KERNEL_SAMPLE_RATE: f32 = 32000.0;
const DOWNSAMPLING_FACTOR: usize = 2;
/// The stmlib global generator's initial state.
pub(crate) const SOURCE_RANDOM_SEED: u32 = 0x21;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Granular {
    pub(crate) overlap: f32,
    pub(crate) window_shape: f32,
    pub(crate) stereo_spread: f32,
    pub(crate) use_deterministic_seed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Spectral {
    pub(crate) quantization: f32,
    pub(crate) refresh_rate: f32,
    pub(crate) phase_randomization: f32,
    pub(crate) warp: f32,
}

/// `clouds::Parameters`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Parameters {
    pub(crate) position: f32,
    pub(crate) size: f32,
    pub(crate) pitch: f32,
    pub(crate) density: f32,
    pub(crate) texture: f32,
    pub(crate) dry_wet: f32,
    pub(crate) stereo_spread: f32,
    pub(crate) feedback: f32,
    pub(crate) reverb: f32,
    pub(crate) freeze: bool,
    pub(crate) trigger: bool,
    pub(crate) gate: bool,
    pub(crate) granular: Granular,
    pub(crate) spectral: Spectral,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PlaybackMode {
    #[default]
    Granular,
    Stretch,
    LoopingDelay,
    Spectral,
    /// `PLAYBACK_MODE_LAST`: nothing prepared yet.
    Last,
}

impl PlaybackMode {
    pub(crate) fn from_index(index: i32) -> Self {
        match index {
            1 => Self::Stretch,
            2 => Self::LoopingDelay,
            3 => Self::Spectral,
            _ => Self::Granular,
        }
    }
}

type Frame = [f32; 2];

/// `clouds::GranularProcessor` over a preallocated image of the firmware's
/// two static memory blocks.
#[derive(Debug)]
pub(crate) struct GranularProcessor {
    memory: Memory,
    pub(crate) parameters: Parameters,
    random: Random,
    guard: Cell<u64>,
    playback_mode: PlaybackMode,
    previous_playback_mode: PlaybackMode,
    num_channels: i32,
    low_fidelity: bool,
    silence: bool,
    bypass: bool,
    reset_buffers: bool,
    freeze_lp: f32,
    dry_wet: f32,
    correlator: Correlator,
    player: GranularSamplePlayer,
    ws_player: WsolaSamplePlayer,
    looper: LoopingSamplePlayer,
    phase_vocoder: PhaseVocoder,
    diffuser: Diffuser,
    reverb: Reverb,
    pitch_shifter: PitchShifter,
    fb_filter: [Svf; 2],
    hp_filter: [Svf; 2],
    lp_filter: [Svf; 2],
    buffers: [AudioBuffer; 2],
    input: [Frame; MAX_BLOCK_SIZE],
    input_downsampled: [Frame; MAX_BLOCK_SIZE / DOWNSAMPLING_FACTOR],
    output_downsampled: [Frame; MAX_BLOCK_SIZE / DOWNSAMPLING_FACTOR],
    output: [Frame; MAX_BLOCK_SIZE],
    feedback: [Frame; MAX_BLOCK_SIZE],
    tail_buffer: [[i16; 256]; 2],
    src_down: SampleRateConverter,
    src_up: SampleRateConverter,
}

impl GranularProcessor {
    /// A processor in the firmware's construction state (zeroed static
    /// storage) after `Init`. This is the only allocating call.
    pub(crate) fn new() -> Self {
        let mut processor = Self {
            memory: Memory::new(),
            parameters: Parameters::default(),
            random: Random {
                state: SOURCE_RANDOM_SEED,
            },
            guard: Cell::new(0),
            playback_mode: PlaybackMode::Granular,
            previous_playback_mode: PlaybackMode::Granular,
            num_channels: 0,
            low_fidelity: false,
            silence: false,
            bypass: false,
            reset_buffers: false,
            freeze_lp: 0.0,
            dry_wet: 0.0,
            correlator: Correlator::default(),
            player: GranularSamplePlayer::new(),
            ws_player: WsolaSamplePlayer::default(),
            looper: LoopingSamplePlayer::default(),
            phase_vocoder: PhaseVocoder::new(),
            diffuser: Diffuser::new(),
            reverb: Reverb::new(),
            pitch_shifter: PitchShifter::new(),
            fb_filter: [Svf::default(); 2],
            hp_filter: [Svf::default(); 2],
            lp_filter: [Svf::default(); 2],
            buffers: [AudioBuffer::default(); 2],
            input: [[0.0; 2]; MAX_BLOCK_SIZE],
            input_downsampled: [[0.0; 2]; MAX_BLOCK_SIZE / DOWNSAMPLING_FACTOR],
            output_downsampled: [[0.0; 2]; MAX_BLOCK_SIZE / DOWNSAMPLING_FACTOR],
            output: [[0.0; 2]; MAX_BLOCK_SIZE],
            feedback: [[0.0; 2]; MAX_BLOCK_SIZE],
            tail_buffer: [[0; 256]; 2],
            src_down: SampleRateConverter::new(-(DOWNSAMPLING_FACTOR as i32)),
            src_up: SampleRateConverter::new(DOWNSAMPLING_FACTOR as i32),
        };
        processor.init();
        processor
    }

    /// Return to the construction state without allocating.
    pub(crate) fn reinitialize(&mut self) {
        self.memory.clear();
        self.parameters = Parameters::default();
        self.random = Random {
            state: SOURCE_RANDOM_SEED,
        };
        self.guard.set(0);
        self.playback_mode = PlaybackMode::Granular;
        self.silence = false;
        self.freeze_lp = 0.0;
        self.correlator = Correlator::default();
        self.player = GranularSamplePlayer::new();
        self.ws_player = WsolaSamplePlayer::default();
        self.looper = LoopingSamplePlayer::default();
        self.phase_vocoder.reset();
        self.diffuser = Diffuser::new();
        self.reverb = Reverb::new();
        self.pitch_shifter = PitchShifter::new();
        self.fb_filter = [Svf::default(); 2];
        self.hp_filter = [Svf::default(); 2];
        self.lp_filter = [Svf::default(); 2];
        self.buffers = [AudioBuffer::default(); 2];
        self.input = [[0.0; 2]; MAX_BLOCK_SIZE];
        self.input_downsampled = [[0.0; 2]; MAX_BLOCK_SIZE / DOWNSAMPLING_FACTOR];
        self.output_downsampled = [[0.0; 2]; MAX_BLOCK_SIZE / DOWNSAMPLING_FACTOR];
        self.output = [[0.0; 2]; MAX_BLOCK_SIZE];
        self.feedback = [[0.0; 2]; MAX_BLOCK_SIZE];
        self.tail_buffer = [[0; 256]; 2];
        self.init();
    }

    /// Same-rate state adoption: copies the complete runtime, buffer audio
    /// included, without allocating.
    pub(crate) fn copy_from(&mut self, other: &Self) {
        self.memory.copy_from(&other.memory);
        self.parameters = other.parameters;
        self.random = other.random;
        self.guard.set(other.guard.get());
        self.playback_mode = other.playback_mode;
        self.previous_playback_mode = other.previous_playback_mode;
        self.num_channels = other.num_channels;
        self.low_fidelity = other.low_fidelity;
        self.silence = other.silence;
        self.bypass = other.bypass;
        self.reset_buffers = other.reset_buffers;
        self.freeze_lp = other.freeze_lp;
        self.dry_wet = other.dry_wet;
        self.correlator = other.correlator;
        self.player.clone_from(&other.player);
        self.ws_player = other.ws_player;
        self.looper = other.looper;
        self.phase_vocoder.copy_from(&other.phase_vocoder);
        self.diffuser = other.diffuser;
        self.reverb = other.reverb;
        self.pitch_shifter = other.pitch_shifter;
        self.fb_filter = other.fb_filter;
        self.hp_filter = other.hp_filter;
        self.lp_filter = other.lp_filter;
        self.buffers = other.buffers;
        self.input = other.input;
        self.input_downsampled = other.input_downsampled;
        self.output_downsampled = other.output_downsampled;
        self.output = other.output;
        self.feedback = other.feedback;
        self.tail_buffer = other.tail_buffer;
        self.src_down = other.src_down;
        self.src_up = other.src_up;
    }

    fn init(&mut self) {
        self.num_channels = 2;
        self.low_fidelity = false;
        self.bypass = false;
        self.src_down.init();
        self.src_up.init();
        self.reset_filters();
        self.previous_playback_mode = PlaybackMode::Last;
        self.reset_buffers = true;
        self.dry_wet = 0.0;
    }

    /// Seed the source random generator (the firmware's global starts at
    /// 0x21). Used by the reference probe; the host keeps the source seed.
    #[allow(dead_code)]
    pub(crate) fn seed_random(&mut self, seed: u32) {
        self.random = Random { state: seed };
    }

    /// Out-of-range reads the source would make (undefined behaviour there).
    pub(crate) fn guarded_reads(&self) -> u64 {
        self.guard.get() + self.memory.guarded.get()
    }

    pub(crate) fn playback_mode(&self) -> PlaybackMode {
        self.playback_mode
    }

    pub(crate) fn set_playback_mode(&mut self, mode: PlaybackMode) {
        self.playback_mode = mode;
    }

    /// `set_quality`: bit 0 selects mono, bit 1 the 8-bit mu-law / 16 kHz
    /// low-fidelity buffer. A change requests a buffer reset in `prepare`.
    pub(crate) fn set_quality(&mut self, quality: i32) {
        let num_channels = if quality & 1 != 0 { 1 } else { 2 };
        self.reset_buffers = self.reset_buffers || self.num_channels != num_channels;
        self.num_channels = num_channels;
        let low_fidelity = quality >> 1 != 0;
        self.reset_buffers = self.reset_buffers || low_fidelity != self.low_fidelity;
        self.low_fidelity = low_fidelity;
    }

    pub(crate) fn quality(&self) -> i32 {
        let mut quality = 0;
        if self.num_channels == 1 {
            quality |= 1;
        }
        if self.low_fidelity {
            quality |= 2;
        }
        quality
    }

    /// Grains sounding in granular mode.
    pub(crate) fn active_grains(&self) -> usize {
        if self.playback_mode == PlaybackMode::Granular {
            self.player.active_grains()
        } else {
            0
        }
    }

    fn resolution_is_8_bit(&self) -> bool {
        self.low_fidelity
    }

    fn sample_rate(&self) -> f32 {
        32000.0
            / if self.low_fidelity {
                DOWNSAMPLING_FACTOR as f32
            } else {
                1.0
            }
    }

    fn reset_filters(&mut self) {
        for i in 0..2 {
            self.fb_filter[i].init();
            self.lp_filter[i].init();
            self.hp_filter[i].init();
        }
    }

    /// `ProcessGranular` on `frames` input, writing `out`.
    fn process_granular(&mut self, low_fidelity: bool) {
        let (input, out): (&[Frame], &mut [Frame]) = if low_fidelity {
            (&self.input_downsampled, &mut self.output_downsampled)
        } else {
            (&self.input, &mut self.output)
        };
        // All modes but the spectral one record into the buffer.
        if self.playback_mode != PlaybackMode::Spectral {
            for i in 0..self.num_channels as usize {
                self.buffers[i].write_fade(
                    &mut self.memory,
                    &mut self.tail_buffer[i],
                    input,
                    i,
                    !self.parameters.freeze,
                );
            }
        }
        let p = &mut self.parameters;
        match self.playback_mode {
            PlaybackMode::Granular => {
                // In Granular mode, DENSITY and TEXTURE are meta parameters.
                p.granular.use_deterministic_seed = p.density < 0.5;
                p.granular.overlap = if p.density >= 0.53 {
                    (p.density - 0.53) * 2.12
                } else if p.density <= 0.47 {
                    (0.47 - p.density) * 2.12
                } else {
                    0.0
                };
                p.granular.window_shape = if p.texture < 0.75 {
                    p.texture * 1.333
                } else {
                    1.0
                };
                self.player.play(
                    &self.buffers,
                    &self.memory,
                    p,
                    &mut self.random,
                    out,
                    &self.guard,
                );
            }
            PlaybackMode::Stretch => {
                self.ws_player
                    .play(&self.buffers, &self.memory, &mut self.correlator, p, out);
            }
            PlaybackMode::LoopingDelay => {
                self.looper.play(&self.buffers, &self.memory, p, out);
            }
            PlaybackMode::Spectral => {
                p.spectral.quantization = p.texture;
                p.spectral.refresh_rate = 0.01 + 0.99 * p.density;
                let warp = p.size - 0.5;
                p.spectral.warp = 4.0 * warp * warp * warp + 0.5;
                let mut randomization = p.density - 0.5;
                randomization = randomization * (randomization * 4.2);
                randomization -= 0.05;
                p.spectral.phase_randomization = randomization.clamp(0.0, 1.0);
                out.copy_from_slice(input);
                self.phase_vocoder.process(&mut self.memory, out);
                if self.num_channels == 1 {
                    for frame in out.iter_mut() {
                        frame[1] = frame[0];
                    }
                }
            }
            PlaybackMode::Last => {}
        }
    }

    /// `Process`: one 32-frame codec block (the audio interrupt).
    pub(crate) fn process(
        &mut self,
        input: &[[i16; 2]; MAX_BLOCK_SIZE],
        output: &mut [[i16; 2]; MAX_BLOCK_SIZE],
    ) {
        let size = MAX_BLOCK_SIZE;
        if self.bypass {
            *output = *input;
            return;
        }
        if self.silence || self.reset_buffers || self.previous_playback_mode != self.playback_mode {
            *output = [[0; 2]; MAX_BLOCK_SIZE];
            return;
        }

        // Convert input buffers to float, and mixdown for mono processing.
        for (frame, codec) in self.input.iter_mut().zip(input.iter()) {
            frame[0] = codec[0] as f32 / 32768.0;
            frame[1] = codec[1] as f32 / 32768.0;
        }
        if self.num_channels == 1 {
            for frame in self.input.iter_mut() {
                frame[0] = (frame[0] + frame[1]) * 0.5;
                frame[1] = frame[0];
            }
        }

        // Apply feedback, with high-pass filtering to prevent build-ups at
        // very low frequencies (causing large DC swings).
        let target = if self.parameters.freeze { 1.0 } else { 0.0 };
        self.freeze_lp += 0.0005 * (target - self.freeze_lp);
        let feedback = self.parameters.feedback;
        let cutoff = (20.0 + 100.0 * feedback * feedback) / self.sample_rate();
        self.fb_filter[0].set_f_q_fast(cutoff, 1.0);
        let reference = self.fb_filter[0];
        self.fb_filter[1].set(&reference);
        self.fb_filter[0].process(FilterMode::HighPass, &mut self.feedback, 0);
        self.fb_filter[1].process(FilterMode::HighPass, &mut self.feedback, 1);
        let fb_gain = feedback * (1.0 - self.freeze_lp);
        for (frame, fb) in self.input.iter_mut().zip(self.feedback.iter()) {
            for c in 0..2 {
                frame[c] += fb_gain * (soft_limit(fb_gain * 1.4 * fb[c] + frame[c]) - frame[c]);
            }
        }

        if self.low_fidelity {
            self.src_down
                .process(&self.input, &mut self.input_downsampled);
            self.process_granular(true);
            self.src_up
                .process(&self.output_downsampled, &mut self.output);
        } else {
            self.process_granular(false);
        }

        // Diffusion and pitch-shifting post-processings.
        if self.playback_mode != PlaybackMode::Spectral {
            let texture = self.parameters.texture;
            let diffusion = if self.playback_mode == PlaybackMode::Granular {
                if texture > 0.75 {
                    (texture - 0.75) * 4.0
                } else {
                    0.0
                }
            } else {
                self.parameters.density
            };
            self.diffuser.set_amount(diffusion);
            self.diffuser.process(&mut self.memory, &mut self.output);
        }

        if self.playback_mode == PlaybackMode::LoopingDelay
            && (!self.parameters.freeze || self.looper.synchronized())
        {
            self.pitch_shifter
                .set_ratio(semitones_to_ratio(self.parameters.pitch));
            self.pitch_shifter.set_size(self.parameters.size);
            self.pitch_shifter
                .process(&mut self.memory, &mut self.output);
        }

        // Apply filters.
        if matches!(
            self.playback_mode,
            PlaybackMode::LoopingDelay | PlaybackMode::Stretch
        ) {
            let cutoff = self.parameters.texture;
            let lp_cutoff = (0.5f32
                * semitones_to_ratio((if cutoff < 0.5 { cutoff - 0.5 } else { 0.0 }) * 216.0))
            .clamp(0.0, 0.499);
            let hp_cutoff = (0.25f32
                * semitones_to_ratio((if cutoff < 0.5 { -0.5 } else { cutoff - 1.0 }) * 216.0))
            .clamp(0.0, 0.499);
            let lpq = 1.0 + 3.0 * (1.0 - feedback) * (0.5 - lp_cutoff);
            self.lp_filter[0].set_f_q_fast(lp_cutoff, lpq);
            self.lp_filter[0].process(FilterMode::LowPass, &mut self.output, 0);
            let reference = self.lp_filter[0];
            self.lp_filter[1].set(&reference);
            self.lp_filter[1].process(FilterMode::LowPass, &mut self.output, 1);
            self.hp_filter[0].set_f_q_fast(hp_cutoff, 1.0);
            self.hp_filter[0].process(FilterMode::HighPass, &mut self.output, 0);
            let reference = self.hp_filter[0];
            self.hp_filter[1].set(&reference);
            self.hp_filter[1].process(FilterMode::HighPass, &mut self.output, 1);
        }

        // This is what is fed back. Reverb is not fed back.
        self.feedback = self.output;

        // Apply reverb.
        let mut reverb_amount = self.parameters.reverb * 0.95;
        reverb_amount += feedback * (2.0 - feedback) * self.freeze_lp;
        let reverb_amount = reverb_amount.clamp(0.0, 1.0);
        self.reverb.set_amount(reverb_amount * 0.54);
        self.reverb.set_diffusion(0.7);
        self.reverb.set_time(0.35 + 0.63 * reverb_amount);
        self.reverb.set_input_gain(0.2);
        self.reverb.set_lp(0.6 + 0.37 * feedback);
        self.reverb.process(&mut self.memory, &mut self.output);

        const POST_GAIN: f32 = 1.2;
        let mut value = self.dry_wet;
        let increment = (self.parameters.dry_wet - self.dry_wet) / size as f32;
        for i in 0..size {
            value += increment;
            let fade_in = interpolate(&LUT_XFADE_IN, value, 16.0, &self.guard);
            let fade_out = interpolate(&LUT_XFADE_OUT, value, 16.0, &self.guard);
            let mut l = input[i][0] as f32 / 32768.0 * fade_out;
            let mut r = input[i][1] as f32 / 32768.0 * fade_out;
            l += self.output[i][0] * POST_GAIN * fade_in;
            r += self.output[i][1] * POST_GAIN * fade_in;
            output[i] = [soft_convert(l), soft_convert(r)];
        }
        self.dry_wet = value;
    }

    /// `Prepare`: the firmware main-loop task (buffer layout changes, WSOLA
    /// correlation search, phase-vocoder frames).
    pub(crate) fn prepare(&mut self) {
        let playback_mode_changed = self.previous_playback_mode != self.playback_mode;
        let benign_change = self.previous_playback_mode != PlaybackMode::Spectral
            && self.playback_mode != PlaybackMode::Spectral
            && self.previous_playback_mode != PlaybackMode::Last;

        if !self.reset_buffers && playback_mode_changed && benign_change {
            self.reset_filters();
            self.pitch_shifter.clear(&mut self.memory);
            self.previous_playback_mode = self.playback_mode;
        }

        if (playback_mode_changed && !benign_change) || self.reset_buffers {
            self.parameters.freeze = false;
        }

        if self.reset_buffers || (playback_mode_changed && !benign_change) {
            let (buffer, buffer_size, workspace, workspace_size) = if self.num_channels == 1 {
                // Large block: sample memory; small block: FX workspace.
                ([0, 0], [LARGE_BYTES, 0], SMALL_BASE, SMALL_BYTES)
            } else {
                // Large block: 64k of sample memory + FX workspace; small
                // block: 64k of sample memory.
                (
                    [0, SMALL_BASE],
                    [SMALL_BYTES, SMALL_BYTES],
                    SMALL_BYTES,
                    LARGE_BYTES - SMALL_BYTES,
                )
            };
            let mut allocator = Allocator::new(workspace, workspace_size);
            // The fixed layout always fits the workspace.
            let diffuser = allocator.allocate(4, 2048).unwrap_or(workspace);
            self.diffuser.init(&mut self.memory, diffuser);
            let reverb = allocator.allocate(2, 16384).unwrap_or(workspace);
            self.reverb.init(&mut self.memory, reverb);
            let correlator_block_size = (MAX_WSOLA_SIZE as usize / 32) + 2;
            let correlator_data = allocator
                .allocate(4, correlator_block_size * 3)
                .unwrap_or(workspace);
            self.correlator
                .init(correlator_data, correlator_data + 4 * correlator_block_size);
            // The pitch shifter deliberately shares the correlator's words.
            self.pitch_shifter.init(&mut self.memory, correlator_data);

            if self.playback_mode == PlaybackMode::Spectral {
                self.phase_vocoder.init(
                    &mut self.memory,
                    buffer,
                    buffer_size,
                    self.num_channels as usize,
                );
            } else {
                let mu_law = self.resolution_is_8_bit();
                for i in 0..self.num_channels as usize {
                    let samples = if mu_law {
                        buffer_size[i]
                    } else {
                        buffer_size[i] >> 1
                    };
                    self.buffers[i].init(&mut self.memory, buffer[i], samples as i32, mu_law);
                }
                let num_grains = ((if self.num_channels == 1 { 40 } else { 32 })
                    * (if self.low_fidelity { 23 } else { 16 }))
                    >> 4;
                self.player.init(self.num_channels, num_grains);
                self.ws_player.init(self.num_channels);
                self.looper.init(self.num_channels);
            }
            self.reset_buffers = false;
            self.previous_playback_mode = self.playback_mode;
        }

        if self.playback_mode == PlaybackMode::Spectral {
            self.phase_vocoder.buffer(
                &mut self.memory,
                &mut self.random,
                &self.parameters,
                &self.guard,
            );
        } else if self.playback_mode == PlaybackMode::Stretch {
            self.ws_player
                .load_correlator(&self.buffers, &mut self.memory, &mut self.correlator);
            self.correlator.evaluate_some_candidates(&self.memory);
        }
    }

    /// Recording position as a fraction of the buffer (panel display).
    pub(crate) fn record_position(&self) -> f32 {
        let buffer = &self.buffers[0];
        if self.playback_mode == PlaybackMode::Spectral || buffer.size() <= 0 {
            0.0
        } else {
            buffer.head() as f32 / buffer.size() as f32
        }
    }

    /// Seconds of audio the current buffer holds at its own rate.
    pub(crate) fn buffer_seconds(&self) -> f32 {
        let bytes = if self.num_channels == 1 {
            LARGE_BYTES
        } else {
            SMALL_BYTES
        } as f32;
        let samples = if self.low_fidelity {
            bytes
        } else {
            bytes / 2.0
        };
        samples / self.sample_rate()
    }
}
