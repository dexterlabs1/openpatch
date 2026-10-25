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
// Rust adaptation (OpenPatch String Field) of rings/dsp/part.h/.cc,
// rings/dsp/note_filter.h, rings/dsp/plucker.h and rings/dsp/limiter.h at
// eurorack f4cfa7697095a28d292bff758beb51deb49b5a86: voice allocation, the
// excitation filter, the six resonator models, the reverb send and the output
// limiter.

use super::fm_voice::FmVoice;
use super::resonator::Resonator;
use super::reverb::Reverb;
use super::stmlib::{
    min, semitones_to_ratio, slope, soft_limit, trunc_i32, CosineOscillator, DcBlocker, DelayLine,
    Guard, Random, Svf,
};
use super::string::{KarplusString, StringLines};
use super::{Memory, Patch, PerformanceState, ResonatorModel, KERNEL_SAMPLE_RATE, MAX_BLOCK_SIZE};

pub(super) const MAX_POLYPHONY: i32 = 4;
pub(super) const NUM_STRINGS: usize = (MAX_POLYPHONY * 2) as usize;
/// `a3`: 440 Hz in cycles per sample.
const A3: f32 = 440.0 / KERNEL_SAMPLE_RATE;

/// `rings::NoteFilter`: adaptive lag on the V/oct note.
#[derive(Clone, Copy, Debug)]
struct NoteFilter {
    previous_values: [f32; 4],
    note: f32,
    stable_note: f32,
    delayed_stable_note: DelayLine<16>,
    coefficient: f32,
    stable_coefficient: f32,
    fast_coefficient: f32,
    slow_coefficient: f32,
    lag_coefficient: f32,
}

impl NoteFilter {
    const ZERO: Self = Self {
        previous_values: [0.0; 4],
        note: 0.0,
        stable_note: 0.0,
        delayed_stable_note: DelayLine::ZERO,
        coefficient: 0.0,
        stable_coefficient: 0.0,
        fast_coefficient: 0.0,
        slow_coefficient: 0.0,
        lag_coefficient: 0.0,
    };

    fn init(
        &mut self,
        sample_rate: f32,
        time_constant_fast_edge: f32,
        time_constant_steady_part: f32,
        edge_recovery_time: f32,
        edge_avoidance_delay: f32,
    ) {
        self.fast_coefficient = 1.0 / (time_constant_fast_edge * sample_rate);
        self.slow_coefficient = 1.0 / (time_constant_steady_part * sample_rate);
        self.lag_coefficient = 1.0 / (edge_recovery_time * sample_rate);
        self.delayed_stable_note.init();
        self.delayed_stable_note
            .cursor
            .set_delay(15usize.min((edge_avoidance_delay * sample_rate) as usize));
        self.note = 69.0;
        self.stable_note = 69.0;
        self.coefficient = self.fast_coefficient;
        self.stable_coefficient = self.slow_coefficient;
        self.previous_values = [self.note; 4];
    }

    fn process(&mut self, note: f32, strum: bool) {
        // If there is a sharp change, follow it instantly.
        if (note - self.note).abs() > 0.4 || strum {
            self.note = note;
            self.stable_note = note;
            self.coefficient = self.fast_coefficient;
            self.stable_coefficient = self.slow_coefficient;
            self.previous_values = [note; 4];
        } else {
            // Median filtering of the raw value.
            self.previous_values.rotate_left(1);
            self.previous_values[3] = note;
            let mut sorted = self.previous_values;
            sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let median = 0.5 * (sorted[1] + sorted[2]);
            // Adaptive lag processor.
            self.note += self.coefficient * (median - self.note);
            self.stable_note += self.stable_coefficient * (self.note - self.stable_note);
            self.coefficient += self.lag_coefficient * (self.slow_coefficient - self.coefficient);
            self.stable_coefficient +=
                self.lag_coefficient * (self.lag_coefficient - self.stable_coefficient);
            self.delayed_stable_note.write(self.stable_note);
        }
    }

    fn note(&self) -> f32 {
        self.note
    }

    fn stable_note(&self) -> f32 {
        self.delayed_stable_note
            .cursor
            .read_delay(&self.delayed_stable_note.line)
    }
}

/// `rings::Plucker`: the internal exciter's filtered noise burst.
#[derive(Clone, Copy, Debug)]
struct Plucker {
    svf: Svf,
    comb_filter: DelayLine<256>,
    remaining_samples: usize,
    comb_filter_period: f32,
    comb_filter_gain: f32,
}

impl Plucker {
    const ZERO: Self = Self {
        svf: Svf::ZERO,
        comb_filter: DelayLine::ZERO,
        remaining_samples: 0,
        comb_filter_period: 0.0,
        comb_filter_gain: 0.0,
    };

    /// `Plucker::Init`; the comb gain is left as it was, as in the source.
    fn init(&mut self) {
        self.svf.init();
        self.comb_filter.init();
        self.remaining_samples = 0;
        self.comb_filter_period = 0.0;
    }

    fn trigger(&mut self, frequency: f32, cutoff: f32, position: f32) {
        let ratio = position * 0.9 + 0.05;
        let mut comb_period = 1.0 / frequency * ratio;
        self.remaining_samples = comb_period as usize;
        while comb_period >= 255.0 {
            comb_period *= 0.5;
        }
        self.comb_filter_period = comb_period;
        self.comb_filter_gain = (1.0 - position) * 0.8;
        self.svf.set_f_q_dirty(min(cutoff, 0.499), 1.0);
    }

    fn process(&mut self, out: &mut [f32], random: &mut Random) {
        let comb_gain = self.comb_filter_gain;
        let comb_delay = self.comb_filter_period;
        for sample in out.iter_mut() {
            let mut input = 0.0;
            if self.remaining_samples != 0 {
                input = 2.0 * random.get_float() - 1.0;
                self.remaining_samples -= 1;
            }
            *sample = input + comb_gain * self.comb_filter.read(comb_delay);
            self.comb_filter.write(*sample);
        }
        self.svf.process_low_pass_block(out);
    }
}

/// `rings::Limiter`.
#[derive(Clone, Copy, Debug)]
struct Limiter {
    peak: f32,
}

impl Limiter {
    fn init(&mut self) {
        self.peak = 0.5;
    }

    fn process(&mut self, l: &mut [f32], r: &mut [f32], pre_gain: f32) {
        for (l, r) in l.iter_mut().zip(r.iter_mut()) {
            let l_pre = *l * pre_gain;
            let r_pre = *r * pre_gain;
            let l_peak = l_pre.abs();
            let r_peak = r_pre.abs();
            let s_peak = (r_pre - l_pre).abs();
            let peak = super::stmlib::max(super::stmlib::max(l_peak, r_peak), s_peak);
            slope(&mut self.peak, peak, 0.05, 0.00002);
            // Clamp to 8 Vpp, clipping softly towards 10 Vpp.
            let gain = if self.peak <= 1.0 {
                1.0
            } else {
                1.0 / self.peak
            };
            *l = soft_limit(l_pre * gain * 0.8);
            *r = soft_limit(r_pre * gain * 0.8);
        }
    }
}

/// The chord table selected by `BRYAN_CHORDS`, which the pinned
/// `rings/dsp/dsp.h` defines; unlisted entries are zero.
#[rustfmt::skip]
static CHORDS: [[[f32; 8]; 11]; MAX_POLYPHONY as usize] = [
    [
        [-12.0, -0.01, 0.0, 0.01, 0.02, 11.98, 11.99, 12.0], // OCT
        [-12.0, -5.0, 0.0, 6.99, 7.0, 11.99, 12.0, 19.0], // 5
        [-12.0, -5.0, 0.0, 5.0, 7.0, 11.99, 12.0, 17.0], // sus4
        [-12.0, -5.0, 0.0, 3.0, 7.0, 3.01, 12.0, 19.0], // m
        [-12.0, -5.0, 0.0, 3.0, 7.0, 3.01, 10.0, 19.0], // m7
        [-12.0, -5.0, 0.0, 3.0, 14.0, 3.01, 10.0, 19.0], // m9
        [-12.0, -5.0, 0.0, 3.0, 7.0, 3.01, 10.0, 17.0], // m11
        [-12.0, -5.0, 0.0, 2.0, 7.0, 9.0, 16.0, 19.0], // 69
        [-12.0, -5.0, 0.0, 4.0, 7.0, 11.0, 14.0, 19.0], // M9
        [-12.0, -5.0, 0.0, 4.0, 7.0, 11.0, 10.99, 19.0], // M7
        [-12.0, -5.0, 0.0, 4.0, 7.0, 11.99, 12.0, 19.0], // M
    ],
    [
        [-12.0, 0.0, 0.01, 12.0, 0.0, 0.0, 0.0, 0.0], // OCT
        [-12.0, 6.99, 7.0, 12.0, 0.0, 0.0, 0.0, 0.0], // 5
        [-12.0, 5.0, 7.0, 12.0, 0.0, 0.0, 0.0, 0.0], // sus4
        [-12.0, 3.0, 11.99, 12.0, 0.0, 0.0, 0.0, 0.0], // m
        [-12.0, 3.0, 10.0, 12.0, 0.0, 0.0, 0.0, 0.0], // m7
        [-12.0, 3.0, 10.0, 14.0, 0.0, 0.0, 0.0, 0.0], // m9
        [-12.0, 3.0, 10.0, 17.0, 0.0, 0.0, 0.0, 0.0], // m11
        [-12.0, 2.0, 9.0, 16.0, 0.0, 0.0, 0.0, 0.0], // 69
        [-12.0, 4.0, 11.0, 14.0, 0.0, 0.0, 0.0, 0.0], // M9
        [-12.0, 4.0, 7.0, 11.0, 0.0, 0.0, 0.0, 0.0], // M7
        [-12.0, 4.0, 7.0, 12.0, 0.0, 0.0, 0.0, 0.0], // M
    ],
    [
        [0.0, -12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 7.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 11.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [-12.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ],
    [
        [0.0, -12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 7.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 11.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [-12.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ],
];

const PING_PATTERN: [i32; 8] = [1, 0, 2, 1, 0, 2, 1, 0];

const MODEL_GAINS: [f32; 6] = [1.4, 1.0, 1.4, 0.7, 1.0, 1.4];

#[inline]
fn squash(mut x: f32) -> f32 {
    if x < 0.5 {
        x *= 2.0;
        x *= x;
        x *= x;
        x *= x;
        x *= x;
        x *= 0.5;
    } else {
        x = 2.0 - 2.0 * x;
        x *= x;
        x *= x;
        x *= x;
        x *= x;
        x = 1.0 - 0.5 * x;
    }
    x
}

/// `rings::Part`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Part {
    bypass: bool,
    dirty: bool,
    model: ResonatorModel,
    active_voice: i32,
    step_counter: u32,
    polyphony: i32,
    resonator: [Resonator; MAX_POLYPHONY as usize],
    string: [KarplusString; NUM_STRINGS],
    lfo: [CosineOscillator; NUM_STRINGS],
    fm_voice: [FmVoice; MAX_POLYPHONY as usize],
    excitation_filter: [Svf; MAX_POLYPHONY as usize],
    dc_blocker: [DcBlocker; MAX_POLYPHONY as usize],
    plucker: [Plucker; MAX_POLYPHONY as usize],
    note: [f32; MAX_POLYPHONY as usize],
    note_filter: NoteFilter,
    resonator_input: [f32; MAX_BLOCK_SIZE],
    sympathetic_resonator_input: [f32; MAX_BLOCK_SIZE],
    noise_burst_buffer: [f32; MAX_BLOCK_SIZE],
    out_buffer: [f32; MAX_BLOCK_SIZE],
    aux_buffer: [f32; MAX_BLOCK_SIZE],
    reverb: Reverb,
    limiter: Limiter,
}

fn lines(memory: &mut Memory, index: usize) -> StringLines<'_> {
    StringLines {
        string: &mut memory.string_lines[index],
        stretch: &mut memory.stretch_lines[index],
    }
}

impl Part {
    /// Zero-initialized static storage, as the firmware's global `Part`.
    pub(super) const ZERO: Self = Self {
        bypass: false,
        dirty: false,
        model: ResonatorModel::Modal,
        active_voice: 0,
        step_counter: 0,
        polyphony: 0,
        resonator: [Resonator::ZERO; MAX_POLYPHONY as usize],
        string: [KarplusString::ZERO; NUM_STRINGS],
        lfo: [CosineOscillator::ZERO; NUM_STRINGS],
        fm_voice: [FmVoice::ZERO; MAX_POLYPHONY as usize],
        excitation_filter: [Svf::ZERO; MAX_POLYPHONY as usize],
        dc_blocker: [DcBlocker::ZERO; MAX_POLYPHONY as usize],
        plucker: [Plucker::ZERO; MAX_POLYPHONY as usize],
        note: [0.0; MAX_POLYPHONY as usize],
        note_filter: NoteFilter::ZERO,
        resonator_input: [0.0; MAX_BLOCK_SIZE],
        sympathetic_resonator_input: [0.0; MAX_BLOCK_SIZE],
        noise_burst_buffer: [0.0; MAX_BLOCK_SIZE],
        out_buffer: [0.0; MAX_BLOCK_SIZE],
        aux_buffer: [0.0; MAX_BLOCK_SIZE],
        reverb: Reverb::ZERO,
        limiter: Limiter { peak: 0.0 },
    };

    pub(super) fn init(&mut self, reverb_buffer: &mut [u16]) {
        self.active_voice = 0;
        self.note = [0.0; MAX_POLYPHONY as usize];
        self.bypass = false;
        self.polyphony = 1;
        self.model = ResonatorModel::Modal;
        self.dirty = true;
        for i in 0..MAX_POLYPHONY as usize {
            self.excitation_filter[i].init();
            self.plucker[i].init();
            self.dc_blocker[i].init(1.0 - 10.0 / KERNEL_SAMPLE_RATE);
        }
        self.reverb.init(reverb_buffer);
        self.limiter.init();
        self.note_filter.init(
            KERNEL_SAMPLE_RATE / MAX_BLOCK_SIZE as f32,
            0.001, // Lag time with a sharp edge on the V/Oct input or trigger.
            0.010, // Lag time after the trigger has been received.
            0.050, // Time to transition from reactive to filtered.
            0.004, // Prevent a sharp edge to partly leak on the previous voice.
        );
    }

    pub(super) fn polyphony(&self) -> i32 {
        self.polyphony
    }

    pub(super) fn set_polyphony(&mut self, polyphony: i32) {
        let old_polyphony = self.polyphony;
        self.polyphony = polyphony.min(MAX_POLYPHONY);
        for i in old_polyphony.max(0)..self.polyphony {
            self.note[i as usize] = self.note[0] + i as f32 * 0.05;
        }
        self.dirty = true;
    }

    pub(super) fn model(&self) -> ResonatorModel {
        self.model
    }

    pub(super) fn set_model(&mut self, model: ResonatorModel) {
        if model != self.model {
            self.model = model;
            self.dirty = true;
        }
    }

    pub(super) fn active_voice(&self) -> i32 {
        self.active_voice
    }

    fn configure_resonators(&mut self, memory: &mut Memory) {
        if !self.dirty {
            return;
        }
        match self.model {
            ResonatorModel::Modal => {
                let resolution = 64 / self.polyphony - 4;
                for i in 0..self.polyphony as usize {
                    self.resonator[i].init();
                    self.resonator[i].set_resolution(resolution);
                }
            }
            ResonatorModel::SympatheticString
            | ResonatorModel::String
            | ResonatorModel::SympatheticStringQuantized
            | ResonatorModel::StringAndReverb => {
                // Seven initializers for eight strings: the last is zero.
                let lfo_frequencies: [f32; NUM_STRINGS] =
                    [0.5, 0.4, 0.35, 0.23, 0.211, 0.2, 0.171, 0.0];
                for (i, lfo_frequency) in lfo_frequencies.iter().enumerate() {
                    let has_dispersion = self.model == ResonatorModel::String
                        || self.model == ResonatorModel::StringAndReverb;
                    self.string[i].init(has_dispersion, lines(memory, i));
                    let mut f_lfo = MAX_BLOCK_SIZE as f32 / KERNEL_SAMPLE_RATE;
                    f_lfo *= lfo_frequency;
                    self.lfo[i].init_approximate(f_lfo);
                }
                for i in 0..self.polyphony as usize {
                    self.plucker[i].init();
                }
            }
            ResonatorModel::FmVoice => {
                for i in 0..self.polyphony as usize {
                    self.fm_voice[i].init();
                }
            }
        }
        if self.active_voice >= self.polyphony {
            self.active_voice = 0;
        }
        self.dirty = false;
    }

    fn compute_sympathetic_strings_notes(
        &self,
        tonic: f32,
        note: f32,
        mut parameter: f32,
        destination: &mut [f32; NUM_STRINGS],
        num_strings: usize,
    ) {
        let notes = [
            tonic,
            note - 12.0,
            note - 7.01955,
            note,
            note + 7.01955,
            note + 12.0,
            note + 19.01955,
            note + 24.0,
            note + 24.0,
        ];
        const DETUNINGS: [f32; 4] = [0.013, 0.011, 0.007, 0.017];

        if parameter >= 2.0 {
            // Quantized chords.
            let chord_index = trunc_i32(parameter - 2.0) as usize;
            let chord = &CHORDS[self.polyphony as usize - 1][chord_index];
            for i in 0..num_strings {
                destination[i] = chord[i] + note;
            }
            return;
        }

        let num_detuned_strings = (num_strings - 1) >> 1;
        let first_detuned_string = num_strings - num_detuned_strings;

        for i in 0..first_detuned_string {
            let mut note = 3.0f32;
            if i != 0 {
                note = parameter * 7.0;
                parameter += (1.0 - parameter) * 0.2;
            }
            let note_integral = trunc_i32(note);
            let note_fractional = squash(note - note_integral as f32);
            let a = notes[note_integral as usize];
            let b = notes[note_integral as usize + 1];
            let note = a + (b - a) * note_fractional;
            destination[i] = note;
            if i + first_detuned_string < num_strings {
                destination[i + first_detuned_string] = destination[i] + DETUNINGS[i & 3];
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_modal_voice(
        &mut self,
        voice: usize,
        performance_state: &PerformanceState,
        patch: &Patch,
        frequency: f32,
        filter_cutoff: f32,
        size: usize,
        guard: &Guard,
    ) {
        // Internal exciter is a pulse, pre-filter.
        if performance_state.internal_exciter
            && voice as i32 == self.active_voice
            && performance_state.strum
        {
            self.resonator_input[0] += 0.25
                * semitones_to_ratio(filter_cutoff * filter_cutoff * 24.0, guard)
                / filter_cutoff;
        }
        // Process through filter.
        self.excitation_filter[voice].process_low_pass_block(&mut self.resonator_input[..size]);

        let r = &mut self.resonator[voice];
        r.set_frequency(frequency);
        r.set_structure(patch.structure);
        r.set_brightness(patch.brightness * patch.brightness);
        r.set_position(patch.position);
        r.set_damping(patch.damping);
        r.process(
            &self.resonator_input[..size],
            &mut self.out_buffer[..size],
            &mut self.aux_buffer[..size],
            guard,
        );
    }

    fn render_fm_voice(
        &mut self,
        voice: usize,
        performance_state: &PerformanceState,
        patch: &Patch,
        frequency: f32,
        size: usize,
        guard: &Guard,
    ) {
        let v = &mut self.fm_voice[voice];
        if performance_state.internal_exciter
            && voice as i32 == self.active_voice
            && performance_state.strum
        {
            v.trigger_internal_envelope();
        }
        v.set_frequency(frequency);
        v.set_ratio(patch.structure);
        v.set_brightness(patch.brightness);
        v.set_feedback_amount(patch.position);
        v.set_position(0.0);
        v.set_damping(patch.damping);
        v.process(
            &self.resonator_input[..size],
            &mut self.out_buffer[..size],
            &mut self.aux_buffer[..size],
            guard,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_string_voice(
        &mut self,
        voice: usize,
        performance_state: &PerformanceState,
        patch: &Patch,
        frequency: f32,
        filter_cutoff: f32,
        size: usize,
        memory: &mut Memory,
        random: &mut Random,
        guard: &Guard,
    ) {
        // Compute number of strings and frequency.
        let mut num_strings = 1usize;
        let mut frequencies = [0.0f32; NUM_STRINGS];

        if self.model == ResonatorModel::SympatheticString
            || self.model == ResonatorModel::SympatheticStringQuantized
        {
            num_strings = (2 * MAX_POLYPHONY / self.polyphony) as usize;
            let parameter = if self.model == ResonatorModel::SympatheticString {
                patch.structure
            } else {
                2.0 + performance_state.chord as f32
            };
            self.compute_sympathetic_strings_notes(
                performance_state.tonic + performance_state.fm,
                performance_state.tonic + self.note[voice] + performance_state.fm,
                parameter,
                &mut frequencies,
                num_strings,
            );
            for frequency in frequencies.iter_mut().take(num_strings) {
                *frequency = semitones_to_ratio(*frequency - 69.0, guard) * A3;
            }
        } else {
            frequencies[0] = frequency;
        }

        if voice as i32 == self.active_voice {
            let gain = 1.0 / (num_strings as f32 * 2.0).sqrt();
            for sample in &mut self.resonator_input[..size] {
                *sample *= gain;
            }
        }

        // Process external input.
        self.excitation_filter[voice].process_low_pass_block(&mut self.resonator_input[..size]);

        // Add noise burst.
        if performance_state.internal_exciter {
            if voice as i32 == self.active_voice && performance_state.strum {
                self.plucker[voice].trigger(frequency, filter_cutoff * 8.0, patch.position);
            }
            self.plucker[voice].process(&mut self.noise_burst_buffer[..size], random);
            for i in 0..size {
                self.resonator_input[i] += self.noise_burst_buffer[i];
            }
        }
        self.dc_blocker[voice].process(&mut self.resonator_input[..size]);

        self.out_buffer[..size].fill(0.0);
        self.aux_buffer[..size].fill(0.0);

        let structure = patch.structure;
        let dispersion = if structure < 0.24 {
            (structure - 0.24) * 4.166
        } else if structure > 0.26 {
            (structure - 0.26) * 1.35135
        } else {
            0.0
        };

        for (string, &string_frequency) in frequencies.iter().enumerate().take(num_strings) {
            let i = voice + string * self.polyphony as usize;
            let lfo_value = self.lfo[i].next();

            let mut brightness = patch.brightness;
            let mut damping = patch.damping;
            let mut position = patch.position;
            let mut glide = 1.0f32;
            let string_index = string as f32 / num_strings as f32;
            let mut sympathetic = false;

            if self.model == ResonatorModel::StringAndReverb {
                damping *= 2.0 - damping;
            }

            // With the internal exciter, string 0 is the main source and the
            // others vibrate by sympathetic resonance; without it, all strings
            // vibrate by sympathetic resonance.
            if string > 0 && performance_state.internal_exciter {
                brightness *= 2.0 - brightness;
                brightness *= 2.0 - brightness;
                damping = 0.7 + patch.damping * 0.27;
                let amount = (0.5 - (0.5 - patch.position).abs()) * 0.9;
                position = patch.position + lfo_value * amount;
                glide = semitones_to_ratio((brightness - 1.0) * 36.0, guard);
                sympathetic = true;
            }

            let s = &mut self.string[i];
            s.set_dispersion(dispersion);
            s.set_frequency(string_frequency, glide);
            s.set_brightness(brightness);
            s.set_position(position);
            s.set_damping(damping + string_index * (0.95 - damping));
            let input = if sympathetic {
                &self.sympathetic_resonator_input[..size]
            } else {
                &self.resonator_input[..size]
            };
            s.process(
                lines(memory, i),
                input,
                &mut self.out_buffer[..size],
                &mut self.aux_buffer[..size],
                random,
                guard,
            );

            if string == 0 {
                // Was 0.1f, Ben Wilson -> 0.2f
                let gain = 0.2 / num_strings as f32;
                for i in 0..size {
                    let sum = self.out_buffer[i] - self.aux_buffer[i];
                    self.sympathetic_resonator_input[i] = gain * sum;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn process(
        &mut self,
        performance_state: &PerformanceState,
        patch: &Patch,
        input: &[f32],
        out: &mut [f32],
        aux: &mut [f32],
        memory: &mut Memory,
        random: &mut Random,
        guard: &Guard,
    ) {
        let size = input.len();
        // Copy inputs to outputs when bypass mode is enabled.
        if self.bypass {
            out.copy_from_slice(input);
            aux.copy_from_slice(input);
            return;
        }

        self.configure_resonators(memory);

        self.note_filter
            .process(performance_state.note, performance_state.strum);

        if performance_state.strum {
            self.note[self.active_voice as usize] = self.note_filter.stable_note();
            if self.polyphony > 1 && self.polyphony & 1 != 0 {
                self.active_voice = PING_PATTERN[(self.step_counter % 8) as usize];
                self.step_counter = (self.step_counter + 1) % 8;
            } else {
                self.active_voice = (self.active_voice + 1) % self.polyphony;
            }
        }

        self.note[self.active_voice as usize] = self.note_filter.note();

        out.fill(0.0);
        aux.fill(0.0);
        for voice in 0..self.polyphony as usize {
            // MIDI note, frequency and excitation filter cutoff.
            let cutoff = patch.brightness * (2.0 - patch.brightness);
            let note = self.note[voice] + performance_state.tonic + performance_state.fm;
            let frequency = semitones_to_ratio(note - 69.0, guard) * A3;
            let filter_cutoff_range = if performance_state.internal_exciter {
                frequency * semitones_to_ratio((cutoff - 0.5) * 96.0, guard)
            } else {
                0.4 * semitones_to_ratio((cutoff - 1.0) * 108.0, guard)
            };
            let active = voice as i32 == self.active_voice;
            let filter_cutoff = min(
                if active {
                    filter_cutoff_range
                } else {
                    10.0 / KERNEL_SAMPLE_RATE
                },
                0.499,
            );
            let filter_q = if performance_state.internal_exciter {
                1.5
            } else {
                0.8
            };

            // Inactive voices receive silence.
            self.excitation_filter[voice].set_f_q_dirty(filter_cutoff, filter_q);
            if active {
                self.resonator_input[..size].copy_from_slice(input);
            } else {
                self.resonator_input[..size].fill(0.0);
            }

            match self.model {
                ResonatorModel::Modal => self.render_modal_voice(
                    voice,
                    performance_state,
                    patch,
                    frequency,
                    filter_cutoff,
                    size,
                    guard,
                ),
                ResonatorModel::FmVoice => {
                    self.render_fm_voice(voice, performance_state, patch, frequency, size, guard)
                }
                _ => self.render_string_voice(
                    voice,
                    performance_state,
                    patch,
                    frequency,
                    filter_cutoff,
                    size,
                    memory,
                    random,
                    guard,
                ),
            }

            if self.polyphony == 1 {
                // Send the two sets of harmonics / pickups to separate outputs.
                for i in 0..size {
                    out[i] += self.out_buffer[i];
                    aux[i] += self.aux_buffer[i];
                }
            } else {
                // Dispatch odd/even voices to separate outputs.
                let destination = if voice & 1 != 0 { &mut *aux } else { &mut *out };
                for (sample, (o, a)) in destination
                    .iter_mut()
                    .zip(self.out_buffer.iter().zip(&self.aux_buffer))
                {
                    *sample += o - a;
                }
            }
        }

        if self.model == ResonatorModel::StringAndReverb {
            for i in 0..size {
                let l = out[i];
                let r = aux[i];
                out[i] = l * patch.position + (1.0 - patch.position) * r;
                aux[i] = r * patch.position + (1.0 - patch.position) * l;
            }
            self.reverb.set_amount(0.1 + patch.damping * 0.5);
            self.reverb.set_diffusion(0.625);
            self.reverb.set_time(0.35 + 0.63 * patch.damping);
            self.reverb.set_input_gain(0.2);
            self.reverb.set_lp(0.3 + patch.brightness * 0.6);
            self.reverb.process(&mut memory.reverb, out, aux);
            for sample in aux.iter_mut() {
                *sample = -*sample;
            }
        }

        // Apply limiter to string output.
        self.limiter
            .process(out, aux, MODEL_GAINS[self.model as usize]);
    }
}
