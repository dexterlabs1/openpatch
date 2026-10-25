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
// Rust adaptation (OpenPatch String Field) of the Mutable Instruments Rings
// resonator: rings/dsp/part.h/.cc, strummer.h and their dependencies at
// eurorack f4cfa7697095a28d292bff758beb51deb49b5a86, plus the DSP portion of
// FillBuffer in rings/rings.cc (exciter-input clearing while normalized, the
// input noise gate, the Strummer and Part calls and the 16-bit output
// conversion). The kernel runs at the source's native 48 kHz in 24-frame
// blocks of 16-bit codec frames.

#[path = "kernel/fm_voice.rs"]
mod fm_voice;
#[path = "kernel/part.rs"]
mod part;
#[path = "kernel/resonator.rs"]
mod resonator;
#[path = "kernel/resources.rs"]
mod resources;
#[path = "kernel/reverb.rs"]
mod reverb;
#[path = "kernel/stmlib.rs"]
mod stmlib;
#[path = "kernel/string.rs"]
mod string;
#[path = "kernel/strummer.rs"]
mod strummer;

use part::{Part, NUM_STRINGS};
use reverb::REVERB_BUFFER_SIZE;
use std::cell::Cell;
use stmlib::{clip16, trunc_i32, Random};
use string::{DELAY_LINE_SIZE, STIFFNESS_LINE_SIZE};
use strummer::Strummer;

pub(crate) const MAX_BLOCK_SIZE: usize = 24;
pub(crate) const KERNEL_SAMPLE_RATE: f32 = 48000.0;
/// Used by the reference probe.
#[allow(dead_code)]
pub(crate) const MAX_POLYPHONY: i32 = part::MAX_POLYPHONY;
pub(crate) const NUM_CHORDS: i32 = 11;
/// The stmlib global generator's initial state.
pub(crate) const SOURCE_RANDOM_SEED: u32 = 0x21;
const NOISE_GATE_THRESHOLD: f32 = 0.00003;

/// `rings::PerformanceState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PerformanceState {
    pub(crate) strum: bool,
    pub(crate) internal_exciter: bool,
    pub(crate) internal_strum: bool,
    pub(crate) internal_note: bool,
    pub(crate) tonic: f32,
    pub(crate) note: f32,
    pub(crate) fm: f32,
    pub(crate) chord: i32,
}

/// `rings::Patch`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Patch {
    pub(crate) structure: f32,
    pub(crate) brightness: f32,
    pub(crate) damping: f32,
    pub(crate) position: f32,
}

/// `rings::ResonatorModel`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ResonatorModel {
    #[default]
    Modal,
    SympatheticString,
    String,
    FmVoice,
    SympatheticStringQuantized,
    StringAndReverb,
}

impl ResonatorModel {
    pub(crate) fn from_index(index: i32) -> Self {
        match index {
            1 => Self::SympatheticString,
            2 => Self::String,
            3 => Self::FmVoice,
            4 => Self::SympatheticStringQuantized,
            5 => Self::StringAndReverb,
            _ => Self::Modal,
        }
    }
}

/// The firmware's large static buffers: eight strings' delay lines and the
/// reverb memory. Allocated once; processing never reallocates.
#[derive(Clone, Debug)]
pub(crate) struct Memory {
    string_lines: Box<[[f32; DELAY_LINE_SIZE]]>,
    stretch_lines: Box<[[f32; STIFFNESS_LINE_SIZE]]>,
    reverb: Box<[u16]>,
}

impl Memory {
    fn new() -> Self {
        Self {
            string_lines: vec![[0.0; DELAY_LINE_SIZE]; NUM_STRINGS].into_boxed_slice(),
            stretch_lines: vec![[0.0; STIFFNESS_LINE_SIZE]; NUM_STRINGS].into_boxed_slice(),
            reverb: vec![0; REVERB_BUFFER_SIZE].into_boxed_slice(),
        }
    }

    fn clear(&mut self) {
        for line in self.string_lines.iter_mut() {
            line.fill(0.0);
        }
        for line in self.stretch_lines.iter_mut() {
            line.fill(0.0);
        }
        self.reverb.fill(0);
    }

    fn copy_from(&mut self, other: &Self) {
        self.string_lines.copy_from_slice(&other.string_lines);
        self.stretch_lines.copy_from_slice(&other.stretch_lines);
        self.reverb.copy_from_slice(&other.reverb);
    }
}

/// Rings' audio path as `FillBuffer` runs it, over zero-initialized storage
/// as the firmware's globals start.
#[derive(Debug)]
pub(crate) struct StringFieldKernel {
    part: Part,
    strummer: Strummer,
    memory: Memory,
    random: Random,
    guard: Cell<u64>,
    in_level: f32,
    input: [f32; MAX_BLOCK_SIZE],
    out: [f32; MAX_BLOCK_SIZE],
    aux: [f32; MAX_BLOCK_SIZE],
    last_strum: bool,
}

impl StringFieldKernel {
    pub(crate) fn new() -> Self {
        let mut kernel = Self {
            part: Part::ZERO,
            strummer: Strummer::ZERO,
            memory: Memory::new(),
            random: Random {
                state: SOURCE_RANDOM_SEED,
            },
            guard: Cell::new(0),
            in_level: 0.0,
            input: [0.0; MAX_BLOCK_SIZE],
            out: [0.0; MAX_BLOCK_SIZE],
            aux: [0.0; MAX_BLOCK_SIZE],
            last_strum: false,
        };
        kernel.init();
        kernel
    }

    /// `Init()` in rings.cc: the Strummer, then the Part.
    fn init(&mut self) {
        self.strummer
            .init(0.01, KERNEL_SAMPLE_RATE / MAX_BLOCK_SIZE as f32);
        self.part.init(&mut self.memory.reverb);
    }

    /// Return to the freshly constructed state without allocating.
    pub(crate) fn reinitialize(&mut self) {
        self.part = Part::ZERO;
        self.strummer = Strummer::ZERO;
        self.memory.clear();
        self.random = Random {
            state: SOURCE_RANDOM_SEED,
        };
        self.guard.set(0);
        self.in_level = 0.0;
        self.input = [0.0; MAX_BLOCK_SIZE];
        self.out = [0.0; MAX_BLOCK_SIZE];
        self.aux = [0.0; MAX_BLOCK_SIZE];
        self.last_strum = false;
        self.init();
    }

    pub(crate) fn copy_from(&mut self, other: &Self) {
        self.part = other.part;
        self.strummer = other.strummer;
        self.memory.copy_from(&other.memory);
        self.random = other.random;
        self.guard.set(other.guard.get());
        self.in_level = other.in_level;
        self.input = other.input;
        self.out = other.out;
        self.aux = other.aux;
        self.last_strum = other.last_strum;
    }

    /// Seed the source random generator (the firmware's global starts at
    /// 0x21). Used by the reference probe; the host keeps the source seed.
    #[allow(dead_code)]
    pub(crate) fn seed_random(&mut self, seed: u32) {
        self.random.state = seed;
    }

    pub(crate) fn polyphony(&self) -> i32 {
        self.part.polyphony()
    }

    /// `Part::set_polyphony`; like the firmware buttons, call it only when the
    /// setting changes (it always reinitializes the resonators).
    pub(crate) fn set_polyphony(&mut self, polyphony: i32) {
        self.part.set_polyphony(polyphony);
    }

    pub(crate) fn model(&self) -> ResonatorModel {
        self.part.model()
    }

    pub(crate) fn set_model(&mut self, model: ResonatorModel) {
        self.part.set_model(model);
    }

    pub(crate) fn active_voice(&self) -> i32 {
        self.part.active_voice()
    }

    /// The strum flag as the Strummer left it in the latest block.
    pub(crate) fn last_strum(&self) -> bool {
        self.last_strum
    }

    pub(crate) fn guarded_reads(&self) -> u64 {
        self.guard.get()
    }

    /// One codec block: `input` is the exciter input (the codec's right
    /// channel); `output` receives the odd and even outputs.
    pub(crate) fn process(
        &mut self,
        mut performance_state: PerformanceState,
        patch: &Patch,
        input: &[i16; MAX_BLOCK_SIZE],
        output: &mut [[i16; 2]; MAX_BLOCK_SIZE],
    ) {
        // CvScaler::DetectAudioNormalization clears the input while the
        // exciter input is normalized; then the noise gate.
        for (x, &word) in self.input.iter_mut().zip(input.iter()) {
            let word = if performance_state.internal_exciter {
                0
            } else {
                word
            };
            let in_sample = word as f32 / 32768.0;
            let error = in_sample * in_sample - self.in_level;
            self.in_level += error * if error > 0.0 { 0.1 } else { 0.0001 };
            let gain = if self.in_level <= NOISE_GATE_THRESHOLD {
                (1.0 / NOISE_GATE_THRESHOLD) * self.in_level
            } else {
                1.0
            };
            *x = gain * in_sample;
        }
        self.strummer.process(&self.input, &mut performance_state);
        self.part.process(
            &performance_state,
            patch,
            &self.input,
            &mut self.out,
            &mut self.aux,
            &mut self.memory,
            &mut self.random,
            &self.guard,
        );
        for (frame, (o, a)) in output.iter_mut().zip(self.out.iter().zip(self.aux.iter())) {
            frame[0] = clip16(trunc_i32(*o * 32768.0)) as i16;
            frame[1] = clip16(trunc_i32(*a * 32768.0)) as i16;
        }
        self.last_strum = performance_state.strum;
    }
}
