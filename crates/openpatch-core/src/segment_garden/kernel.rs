// Copyright 2017 Emilie Gillet.
// Copyright 2012, 2014, 2015 Emilie Gillet (stmlib helpers).
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
// Rust adaptation Copyright 2026 OpenPatch contributors.
// Source: pichenettes/eurorack f4cfa7697095a28d292bff758beb51deb49b5a86,
// stages/segment_generator.{h,cc}, delay_line_16_bits.h and resources.cc;
// stmlib e3bd7c9cc00e4364166f9905c0509b6ffd0535ec (DSP helpers and RNG).
// Preserves source evaluation order and static-storage initialization.
// Explicit extensions: safe LUT endpoint clamping; per-instance seeded RNG;
// full-capacity rounded-RNG address clamping (see RANDOM sequencer branch).
#![allow(clippy::excessive_precision, clippy::manual_clamp)]

#[path = "kernel_resources.rs"]
mod resources;
use crate::tidal_functions::extractor::RampExtractor;
use resources::*;

pub const SAMPLE_RATE: f32 = 31250.0;
pub const MAX_NUM_SEGMENTS: usize = 36;
pub const GATE_HIGH: u8 = 1;
pub const GATE_RISING: u8 = 2;
pub const GATE_FALLING: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SegmentType {
    Ramp = 0,
    Step = 1,
    Hold = 2,
    Alt = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Configuration {
    pub kind: SegmentType,
    pub looped: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Parameters {
    pub primary: f32,
    pub secondary: f32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Output {
    pub value: f32,
    pub phase: f32,
    pub segment: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidSegmentCount,
    UnsupportedAlt,
    InvalidSegmentIndex,
}

// Symbolic references preserve source pointer identity without self-references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ref {
    Zero,
    Half,
    One,
    Primary(usize),
    Secondary(usize),
}
#[derive(Clone, Copy, Debug)]
struct Segment {
    start: Option<Ref>,
    time: Option<Ref>,
    curve: Ref,
    portamento: Ref,
    end: Ref,
    phase: Option<Ref>,
    rising: i32,
    falling: i32,
    complete: i32,
}
impl Default for Segment {
    fn default() -> Self {
        Self {
            start: Some(Ref::Zero),
            time: Some(Ref::Zero),
            curve: Ref::Half,
            portamento: Ref::Zero,
            end: Ref::Zero,
            phase: None,
            rising: 0,
            falling: 0,
            complete: 0,
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Mode {
    Multi,
    Sequencer,
    Decay,
    Pulse,
    Gate,
    SampleHold,
    TapLfo,
    Lfo,
    Delay,
    Portamento,
    Zero,
    Slave,
}
#[derive(Clone, Copy, Debug)]
struct Quantizer {
    steps: i32,
    hysteresis: f32,
    value: i32,
}
impl Quantizer {
    fn new(steps: i32, hysteresis: f32) -> Self {
        Self {
            steps,
            hysteresis,
            value: 0,
        }
    }
    fn process(&mut self, mut value: f32) -> i32 {
        value *= self.steps as f32;
        value += -0.5;
        let sign = if value > self.value as f32 { -1.0 } else { 1.0 };
        let q = (value + sign * self.hysteresis + 0.5) as i32;
        self.value = q.clamp(0, self.steps - 1);
        self.value
    }
}

#[derive(Clone, Debug)]
pub struct SegmentGenerator {
    phase: f32,
    aux: f32,
    start: f32,
    value: f32,
    lp: f32,
    primary: f32,
    active: usize,
    previous: usize,
    monitored: usize,
    retrig_delay: i32,
    num_segments: usize,
    mode: Mode,
    ramp_extractor: RampExtractor,
    function_quantizer: Quantizer,
    address_quantizer: Quantizer,
    segments: [Segment; MAX_NUM_SEGMENTS + 1],
    parameters: [Parameters; MAX_NUM_SEGMENTS],
    delay: [i16; 577],
    delay_write: usize,
    gate_delay: [u8; 128],
    gate_write: usize,
    first_step: usize,
    last_step: usize,
    up_down_counter: i32,
    reset: bool,
    accepted_gate: bool,
    inhibit_clock: i32,
    rng: u32,
    step_quantizers: [Quantizer; MAX_NUM_SEGMENTS],
    step_quantizers_enabled: bool,
    quantized_output: bool,
}
impl Default for SegmentGenerator {
    fn default() -> Self {
        Self::new()
    }
}
impl SegmentGenerator {
    pub fn new() -> Self {
        Self {
            phase: 0.0,
            aux: 0.0,
            start: 0.0,
            value: 0.0,
            lp: 0.0,
            primary: 0.0,
            active: 0,
            previous: 0,
            monitored: 0,
            retrig_delay: 0,
            num_segments: 0,
            mode: Mode::Multi,
            ramp_extractor: RampExtractor::new(SAMPLE_RATE, 1000.0 / SAMPLE_RATE),
            function_quantizer: Quantizer::new(2, 0.025),
            address_quantizer: Quantizer::new(2, 0.025),
            segments: [Segment::default(); MAX_NUM_SEGMENTS + 1],
            parameters: [Parameters::default(); MAX_NUM_SEGMENTS],
            delay: [0; 577],
            delay_write: 0,
            gate_delay: [0; 128],
            gate_write: 0,
            first_step: 1,
            last_step: 1,
            up_down_counter: 0,
            reset: false,
            accepted_gate: true,
            inhibit_clock: 0,
            rng: 0x21,
            step_quantizers: [Quantizer::new(13, 0.03); MAX_NUM_SEGMENTS],
            step_quantizers_enabled: false,
            quantized_output: false,
        }
    }
    /// Mirrors firmware Init(note_quantizer), with owned stage quantizers.
    pub fn new_quantized() -> Self {
        let mut generator = Self::new();
        generator.step_quantizers_enabled = true;
        generator
    }
    /// Native startup extension for a freshly created kernel: advance only
    /// the clock tracker by one LOW source sample. Mode, generator phase and
    /// output history are untouched. Raw new() deliberately remains unprimed.
    pub fn prime_clock_tracker(&mut self) {
        let mut ramp = [0.0];
        self.ramp_extractor
            .process(false, false, 1.0, 1, &[0], &mut ramp);
    }
    pub fn num_segments(&self) -> usize {
        self.num_segments
    }
    pub fn seed_rng(&mut self, seed: u32) {
        self.rng = seed;
    }
    /// State belongs to physical firmware slots, not to envelope groups.
    /// A fixed-slot adapter must load the shared physical slot's state before
    /// processing a block and store this value back afterward. Index is the
    /// stage offset from this generator's physical slot. The firmware uses
    /// 42 shared quantizers so that all 36 offsets of each of its six kernels
    /// can be synchronized without changing ownership on live repatching.
    pub fn note_quantizer_state(&self, index: usize) -> Result<i32, Error> {
        self.step_quantizers
            .get(index)
            .map(|q| q.value)
            .ok_or(Error::InvalidSegmentIndex)
    }
    pub fn set_note_quantizer_state(&mut self, index: usize, value: i32) -> Result<(), Error> {
        let quantizer = self
            .step_quantizers
            .get_mut(index)
            .ok_or(Error::InvalidSegmentIndex)?;
        quantizer.value = value;
        Ok(())
    }
    pub fn rng_state(&self) -> u32 {
        self.rng
    }
    fn random(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        self.rng as f32 / 4294967296.0
    }
    pub fn set_parameters(
        &mut self,
        index: usize,
        primary: f32,
        secondary: f32,
    ) -> Result<(), Error> {
        let p = self
            .parameters
            .get_mut(index)
            .ok_or(Error::InvalidSegmentIndex)?;
        *p = Parameters { primary, secondary };
        Ok(())
    }
    pub fn configure_slave(&mut self, index: usize) -> Result<(), Error> {
        if index >= MAX_NUM_SEGMENTS {
            return Err(Error::InvalidSegmentIndex);
        }
        self.monitored = index;
        self.mode = Mode::Slave;
        self.num_segments = 0;
        Ok(())
    }
    /// Exact ConfigureSingleSegment entrypoint: unlike Configure, this keeps
    /// the existing function quantizer state (including its initialized size).
    pub fn configure_single(
        &mut self,
        has_trigger: bool,
        configuration: Configuration,
    ) -> Result<(), Error> {
        use SegmentType::*;
        if configuration.kind == Alt {
            return Err(Error::UnsupportedAlt);
        }
        self.mode = match (configuration.kind, has_trigger, configuration.looped) {
            (Ramp, false, false) => Mode::Zero,
            (Ramp, false, true) => Mode::Lfo,
            (Ramp, true, false) => Mode::Decay,
            (Ramp, true, true) => Mode::TapLfo,
            (Step, false, _) => Mode::Portamento,
            (Step, true, _) => Mode::SampleHold,
            (Hold, false, _) => Mode::Delay,
            (Hold, true, false) => Mode::Pulse,
            (Hold, true, true) => Mode::Gate,
            _ => unreachable!(),
        };
        self.num_segments = 1;
        Ok(())
    }
    pub fn configure(
        &mut self,
        has_trigger: bool,
        configurations: &[Configuration],
    ) -> Result<(), Error> {
        use SegmentType::*;
        let n = configurations.len();
        if n == 0 || n > MAX_NUM_SEGMENTS {
            return Err(Error::InvalidSegmentCount);
        }
        if configurations.iter().any(|c| c.kind == Alt) {
            return Err(Error::UnsupportedAlt);
        }
        if n == 1 {
            self.function_quantizer = Quantizer::new(7, 0.025);
            return self.configure_single(has_trigger, configurations[0]);
        }
        let sequencer = configurations[0].kind != Step
            && !configurations[0].looped
            && n >= 3
            && configurations[1..].iter().all(|c| c.kind == Step);
        if sequencer {
            self.function_quantizer = Quantizer::new(7, 0.025);
            self.configure_sequencer(configurations);
            return Ok(());
        }
        self.num_segments = n;
        self.mode = Mode::Multi;
        let mut loop_start: i32 = -1;
        let mut loop_end: i32 = -1;
        let mut has_steps = false;
        let mut first_ramp = None;
        for (i, c) in configurations.iter().enumerate() {
            has_steps |= c.kind == Step;
            if c.looped {
                if loop_start == -1 {
                    loop_start = i as i32;
                }
                loop_end = i as i32;
            }
            if c.kind == Ramp && first_ramp.is_none() {
                first_ramp = Some(i);
            }
        }
        let steps_inside = loop_start >= 0
            && configurations[loop_start as usize..=loop_end as usize]
                .iter()
                .any(|c| c.kind == Step);
        for (i, c) in configurations.iter().enumerate() {
            let single_loop = i as i32 == loop_start && i as i32 == loop_end;
            let mut s = self.segments[i];
            if c.kind == Ramp {
                s.start = None;
                s.time = Some(Ref::Primary(i));
                s.curve = Ref::Secondary(i);
                s.portamento = Ref::Zero;
                s.phase = None;
                s.end = if i == n - 1 {
                    Ref::Zero
                } else if configurations[i + 1].kind != Ramp {
                    Ref::Primary(i + 1)
                } else if Some(i) == first_ramp {
                    Ref::One
                } else {
                    s.curve = Ref::Half;
                    Ref::Secondary(i)
                };
            } else {
                s.start = Some(Ref::Primary(i));
                s.end = Ref::Primary(i);
                s.curve = Ref::Half;
                if c.kind == Step {
                    s.portamento = Ref::Secondary(i);
                    s.time = None;
                    s.phase = Some(if single_loop { Ref::Zero } else { Ref::One });
                } else {
                    s.portamento = Ref::Zero;
                    s.time = if single_loop {
                        None
                    } else {
                        Some(Ref::Secondary(i))
                    };
                    s.phase = Some(Ref::One);
                }
            }
            s.complete = if i as i32 == loop_end {
                loop_start
            } else {
                i as i32 + 1
            };
            s.falling = if loop_end == -1 || loop_end == n as i32 - 1 || has_steps {
                -1
            } else {
                loop_end + 1
            };
            s.rising = 0;
            if has_steps {
                if !steps_inside && i as i32 >= loop_start && i as i32 <= loop_end {
                    s.rising = (loop_end + 1) % n as i32;
                } else {
                    let mut follow_loop = loop_end != -1;
                    let mut next = i;
                    while configurations[next].kind != Step {
                        next += 1;
                        if follow_loop && next as i32 == loop_end + 1 {
                            next = loop_start as usize;
                            follow_loop = false;
                        }
                        if next >= n {
                            next = n - 1;
                            break;
                        }
                    }
                    s.rising = if next as i32 == loop_end {
                        loop_start
                    } else {
                        (next as i32 + 1) % n as i32
                    };
                }
            }
            self.segments[i] = s;
        }
        let end = self.segments[n - 1].end;
        let sentinel = &mut self.segments[n];
        sentinel.end = end;
        sentinel.start = Some(end);
        sentinel.time = Some(Ref::Zero);
        sentinel.curve = Ref::Half;
        sentinel.portamento = Ref::Zero;
        sentinel.rising = 0;
        sentinel.falling = -1;
        sentinel.complete = if loop_end == n as i32 - 1 { 0 } else { -1 };
        // Source deliberately does not assign sentinel.phase on reconfiguration.
        self.previous = n;
        self.active = n;
        Ok(())
    }
    fn configure_sequencer(&mut self, configurations: &[Configuration]) {
        self.num_segments = configurations.len();
        self.first_step = 0;
        for (i, c) in configurations.iter().enumerate().skip(1) {
            if c.looped {
                if self.first_step == 0 {
                    self.first_step = i;
                    self.last_step = i;
                } else {
                    self.last_step = i;
                }
            }
        }
        if self.first_step == 0 {
            self.first_step = 1;
            self.last_step = configurations.len() - 1;
        }
        let steps = self.last_step - self.first_step + 1;
        self.address_quantizer = Quantizer::new(steps as i32, 0.02 / 8.0 * steps as f32);
        self.inhibit_clock = 0;
        self.up_down_counter = 0;
        self.reset = false;
        self.quantized_output =
            configurations[0].kind == SegmentType::Ramp && self.step_quantizers_enabled;
        self.lp = 0.0;
        self.value = 0.0;
        self.active = self.first_step;
        self.mode = Mode::Sequencer;
    }
    fn deref(&self, r: Ref) -> f32 {
        match r {
            Ref::Zero => 0.0,
            Ref::Half => 0.5,
            Ref::One => 1.0,
            Ref::Primary(i) => self.parameters[i].primary,
            Ref::Secondary(i) => self.parameters[i].secondary,
        }
    }
    /// Gate flags and output must have identical lengths. For Slave, output is
    /// the already-rendered master buffer; only its value is overwritten.
    /// Raw tapped-LFO startup retains the upstream extractor's frame-zero edge boundary.
    pub fn process(&mut self, gates: &[u8], output: &mut [Output]) {
        assert_eq!(gates.len(), output.len());
        if output.is_empty() {
            return;
        }
        match self.mode {
            Mode::Multi => self.process_multi(gates, output),
            Mode::Sequencer => self.process_sequencer(gates, output),
            Mode::Lfo | Mode::TapLfo => self.process_lfo(gates, output),
            Mode::Slave => {
                for out in output {
                    self.active = if out.segment == self.monitored as i32 {
                        0
                    } else {
                        1
                    };
                    out.value = if self.active != 0 {
                        0.0
                    } else {
                        1.0 - out.phase
                    };
                }
            }
            _ => self.process_single(gates, output),
        }
    }
    fn process_multi(&mut self, gates: &[u8], output: &mut [Output]) {
        let mut phase = self.phase;
        let mut start = self.start;
        let mut lp = self.lp;
        let mut value = self.value;
        for (&gate, out) in gates.iter().zip(output) {
            let segment = self.segments[self.active];
            let previous = self.segments[self.previous];
            if segment.start.is_none() && previous.phase.is_some() && segment.end != previous.end {
                start += portamento(self.deref(previous.portamento))
                    * (self.deref(previous.end) - start);
            }
            if let Some(time) = segment.time {
                phase += frequency(self.deref(time));
            }
            let complete = phase >= 1.0;
            if complete {
                phase = 1.0;
            }
            value = crossfade(
                start,
                self.deref(segment.end),
                warp(
                    segment.phase.map(|p| self.deref(p)).unwrap_or(phase),
                    self.deref(segment.curve),
                ),
            );
            lp += portamento(self.deref(segment.portamento)) * (value - lp);
            let next = if gate & GATE_RISING != 0 {
                segment.rising
            } else if gate & GATE_FALLING != 0 {
                segment.falling
            } else if complete {
                segment.complete
            } else {
                -1
            };
            if next != -1 {
                phase = 0.0;
                let destination = self.segments[next as usize];
                start = destination.start.map(|p| self.deref(p)).unwrap_or(
                    if next as usize == self.active {
                        start
                    } else {
                        value
                    },
                );
                if next as usize != self.active {
                    self.previous = self.active;
                }
                self.active = next as usize;
            }
            *out = Output {
                value: lp,
                phase,
                segment: self.active as i32,
            };
        }
        self.phase = phase;
        self.start = start;
        self.lp = lp;
        self.value = value;
    }
    fn process_single(&mut self, gates: &[u8], output: &mut [Output]) {
        let p = self.parameters[0];
        let f = frequency(if matches!(self.mode, Mode::Decay) {
            p.primary
        } else {
            p.secondary
        });
        let coefficient = portamento(p.secondary);
        let increment = (p.primary - self.primary) / output.len() as f32;
        let interpolated = matches!(
            self.mode,
            Mode::Pulse | Mode::Gate | Mode::SampleHold | Mode::Delay | Mode::Portamento
        );
        let mut delay_time =
            semitones_to_ratio(2.0 * (p.secondary - 0.5) * 36.0) * 0.5 * SAMPLE_RATE;
        let delay_frequency = 1.0 / delay_time;
        let clock_frequency = if delay_time >= 575.0 {
            delay_time = 575.0;
            575.0 * delay_frequency
        } else {
            1.0
        };
        for (&gate, out) in gates.iter().zip(output) {
            if interpolated {
                self.primary += increment;
            }
            match self.mode {
                Mode::Decay => {
                    if gate & GATE_RISING != 0 {
                        self.phase = 0.0;
                        self.active = 0;
                    }
                    self.phase += f;
                    if self.phase >= 1.0 {
                        self.phase = 1.0;
                        self.active = 1;
                    }
                    self.value = 1.0 - warp(self.phase, p.secondary);
                    self.lp = self.value;
                    *out = Output {
                        value: self.lp,
                        phase: self.phase,
                        segment: self.active as i32,
                    };
                }
                Mode::Pulse => {
                    if gate & GATE_RISING != 0 {
                        self.retrig_delay = if self.active == 0 { 32 } else { 0 };
                        self.phase = 0.0;
                        self.active = 0;
                    }
                    if self.retrig_delay != 0 {
                        self.retrig_delay -= 1;
                    }
                    self.phase += f;
                    if self.phase >= 1.0 {
                        self.phase = 1.0;
                        self.active = 1;
                    }
                    self.value = if self.active == 0 && self.retrig_delay == 0 {
                        self.primary
                    } else {
                        0.0
                    };
                    self.lp = self.value;
                    *out = Output {
                        value: self.lp,
                        phase: self.phase,
                        segment: self.active as i32,
                    };
                }
                Mode::Gate => {
                    if gate & GATE_RISING != 0 {
                        self.accepted_gate = self.random() < p.secondary * 1.01;
                    }
                    self.active = if gate & GATE_HIGH != 0 && self.accepted_gate {
                        0
                    } else {
                        1
                    };
                    self.value = if self.active == 0 { self.primary } else { 0.0 };
                    self.lp = self.value;
                    *out = Output {
                        value: self.lp,
                        phase: 0.5,
                        segment: self.active as i32,
                    };
                }
                Mode::SampleHold => {
                    self.gate_delay[self.gate_write] = gate;
                    self.gate_write = (self.gate_write + 127) % 128;
                    if self.gate_delay[(self.gate_write + 62) % 128] & GATE_RISING != 0 {
                        self.value = self.primary;
                    }
                    self.active = if gate & GATE_HIGH != 0 { 0 } else { 1 };
                    self.lp += coefficient * (self.value - self.lp);
                    *out = Output {
                        value: self.lp,
                        phase: 0.5,
                        segment: self.active as i32,
                    };
                }
                Mode::Delay => {
                    self.phase += clock_frequency;
                    self.lp += clock_frequency * (self.primary - self.lp);
                    if self.phase >= 1.0 {
                        self.phase -= 1.0;
                        self.delay_write_sample(self.lp);
                    }
                    self.aux += delay_frequency;
                    if self.aux >= 1.0 {
                        self.aux -= 1.0;
                    }
                    self.active = if self.aux < 0.5 { 0 } else { 1 };
                    self.value +=
                        clock_frequency * (self.delay_read(delay_time - self.phase) - self.value);
                    *out = Output {
                        value: self.value,
                        phase: self.aux,
                        segment: self.active as i32,
                    };
                }
                Mode::Portamento => {
                    self.active = 0;
                    self.value = self.primary;
                    self.lp += coefficient * (self.value - self.lp);
                    *out = Output {
                        value: self.lp,
                        phase: 0.5,
                        segment: 0,
                    };
                }
                Mode::Zero => {
                    self.value = 0.0;
                    self.active = 1;
                    *out = Output {
                        value: 0.0,
                        phase: 0.5,
                        segment: 1,
                    };
                }
                _ => unreachable!(),
            }
        }
    }
    fn delay_write_sample(&mut self, sample: f32) {
        let word = ((sample * 32768.0) as i32).clamp(-32768, 32767) as i16;
        self.delay[self.delay_write] = word;
        if self.delay_write == 0 {
            self.delay[576] = word;
            self.delay_write = 575;
        } else {
            self.delay_write -= 1;
        }
    }
    fn delay_read(&self, delay: f32) -> f32 {
        let integral = delay as i32;
        let fractional = delay - integral as f32;
        let i = (self.delay_write as i32 + integral).rem_euclid(576) as usize;
        let a = self.delay[i] as f32 / 32768.0;
        let b = self.delay[i + 1] as f32 / 32768.0;
        a + (b - a) * fractional
    }
    fn process_lfo(&mut self, gates: &[u8], output: &mut [Output]) {
        let p = self.parameters[0];
        if matches!(self.mode, Mode::TapLfo) {
            const RATIOS: [(f32, i32); 7] = [
                (0.249999, 4),
                (0.333333, 3),
                (0.499999, 2),
                (0.999999, 1),
                (1.999999, 1),
                (2.999999, 1),
                (3.999999, 1),
            ];
            let ratio = RATIOS[self.function_quantizer.process(p.primary * 1.03) as usize];
            for (&gate, out) in gates.iter().zip(output.iter_mut()) {
                let mut ramp = [0.0];
                self.ramp_extractor
                    .process(false, false, ratio.0, ratio.1, &[gate], &mut ramp);
                *out = shape_lfo(p.secondary, ramp[0]);
            }
        } else {
            let f = (96.0 * (p.primary - 0.5)).clamp(-128.0, 127.0);
            let frequency = semitones_to_ratio(f) * 2.0439497 / SAMPLE_RATE;
            for out in output.iter_mut() {
                self.phase += frequency;
                if self.phase >= 1.0 {
                    self.phase -= 1.0;
                }
                *out = shape_lfo(p.secondary, self.phase);
            }
        }
        self.active = output[output.len() - 1].segment as usize;
    }
    fn process_sequencer(&mut self, gates: &[u8], output: &mut [Output]) {
        let direction = self
            .function_quantizer
            .process(self.parameters[0].secondary);
        if direction == 6 {
            self.reset = false;
            self.active = self.address_quantizer.process(self.parameters[0].primary) as usize
                + self.first_step;
        } else {
            if self.parameters[0].primary > 0.125 && !self.reset {
                self.reset = true;
                self.active = if direction == 1 {
                    self.last_step
                } else {
                    self.first_step
                };
                self.up_down_counter = 0;
                self.inhibit_clock = 156;
            }
            if self.reset && self.parameters[0].primary < 0.0625 {
                self.reset = false;
            }
        }
        for (&gate, out) in gates.iter().zip(output) {
            if self.inhibit_clock != 0 {
                self.inhibit_clock -= 1;
            }
            let clockable = self.inhibit_clock == 0 && !self.reset && direction != 6;
            if gate & GATE_RISING != 0 && clockable {
                let n = (self.last_step - self.first_step + 1) as i32;
                match direction {
                    0 => {
                        self.active += 1;
                        if self.active > self.last_step {
                            self.active = self.first_step;
                        }
                    }
                    1 => {
                        if self.active <= self.first_step {
                            self.active = self.last_step;
                        } else {
                            self.active -= 1;
                        }
                    }
                    2 => {
                        if n == 1 {
                            self.active = self.first_step;
                        } else {
                            self.up_down_counter = (self.up_down_counter + 1) % (2 * (n - 1));
                            self.active = self.first_step
                                + if self.up_down_counter < n {
                                    self.up_down_counter
                                } else {
                                    2 * (n - 1) - self.up_down_counter
                                } as usize;
                        }
                    }
                    3 => {
                        if n == 1 {
                            self.active = self.first_step;
                        } else if n == 2 {
                            self.up_down_counter = (self.up_down_counter + 1) % 2;
                            self.active = self.first_step + self.up_down_counter as usize;
                        } else {
                            self.up_down_counter = (self.up_down_counter + 1) % (4 * n - 8);
                            let i = (self.up_down_counter - 1) / 2;
                            self.active = self.first_step
                                + if self.up_down_counter & 1 != 0 {
                                    1 + if i < n - 1 { i } else { 2 * (n - 2) - i }
                                } else {
                                    0
                                } as usize;
                        }
                    }
                    4 => {
                        let selected = self.first_step + (self.random() * n as f32) as usize;
                        // stmlib GetFloat can round to 1.0f. Preserve its
                        // inactive-stage quirk for groups shorter than 36,
                        // but make the full-capacity source out-of-bounds
                        // access a defined last-parameter/last-segment clamp.
                        self.active = selected.min(MAX_NUM_SEGMENTS - 1);
                    }
                    5 => {
                        let r = (self.random() * (n - 1) as f32) as i32;
                        self.active = self.first_step
                            + ((self.active as i32 - self.first_step as i32 + r + 1) % n) as usize;
                    }
                    _ => {}
                }
            }
            self.value = self.parameters[self.active].primary;
            if self.quantized_output {
                self.value = self.step_quantizers[self.active].process(self.value) as f32 / 96.0;
            }
            self.lp += portamento(self.parameters[self.active].secondary) * (self.value - self.lp);
            *out = Output {
                value: self.lp,
                phase: 0.0,
                segment: self.active as i32,
            };
        }
    }
}
fn crossfade(a: f32, b: f32, fade: f32) -> f32 {
    a + (b - a) * fade
}
fn warp(mut t: f32, mut curve: f32) -> f32 {
    curve -= 0.5;
    let flip = curve < 0.0;
    if flip {
        t = 1.0 - t;
    }
    let a = 128.0 * curve * curve;
    t = (1.0 + a) * t / (1.0 + a * t);
    if flip {
        t = 1.0 - t;
    }
    t
}
fn frequency(rate: f32) -> f32 {
    // Upstream CONSTRAIN upper bound equals table length (undefined at rate >=2).
    LUT_ENV_FREQUENCY
        [((rate * 2048.0) as i32).clamp(0, LUT_ENV_FREQUENCY.len() as i32 - 1) as usize]
}
fn portamento(rate: f32) -> f32 {
    // Upstream reads past the table at secondary==1; clamp that endpoint safely.
    LUT_PORTAMENTO_COEFFICIENT
        [((rate * 512.0) as i32).clamp(0, LUT_PORTAMENTO_COEFFICIENT.len() as i32 - 1) as usize]
}
fn semitones_to_ratio(semitones: f32) -> f32 {
    let pitch = semitones + 128.0;
    let integral = pitch as i32;
    let fractional = pitch - integral as f32;
    LUT_PITCH_RATIO_HIGH[integral.clamp(0, 255) as usize]
        * LUT_PITCH_RATIO_LOW[((fractional * 256.0) as i32).clamp(0, 255) as usize]
}
fn shape_lfo(mut shape: f32, input_phase: f32) -> Output {
    shape -= 0.5;
    shape = 2.0 + 9.999999 * shape / (1.0 + 3.0 * shape.abs());
    let slope = (shape * 0.5).min(0.5);
    let plateau_width = (shape - 3.0).max(0.0);
    let sine_amount = (if shape < 2.0 {
        shape - 1.0
    } else {
        3.0 - shape
    })
    .max(0.0);
    let slope_up = 1.0 / slope;
    let slope_down = 1.0 / (1.0 - slope);
    let plateau = 0.5 * (1.0 - plateau_width);
    let normalization = 1.0 / plateau;
    let phase_shift = plateau_width * 0.25;
    let mut phase = input_phase + phase_shift;
    if phase > 1.0 {
        phase -= 1.0;
    }
    let mut triangle = if phase < slope {
        slope_up * phase
    } else {
        1.0 - (phase - slope) * slope_down
    };
    triangle -= 0.5;
    triangle = triangle.clamp(-plateau, plateau);
    triangle *= normalization;
    let mut index = phase + 0.75;
    index -= (index as i32) as f32;
    index *= 1024.0;
    let integral = index as usize;
    let fractional = index - integral as f32;
    let sine = LUT_SINE[integral] + (LUT_SINE[integral + 1] - LUT_SINE[integral]) * fractional;
    Output {
        phase: input_phase,
        value: 0.5 * crossfade(triangle, sine, sine_amount) + 0.5,
        segment: if phase < 0.5 { 0 } else { 1 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_configuration_preserves_running_state() {
        let mut g = SegmentGenerator::new();
        g.configure(
            false,
            &[Configuration {
                kind: SegmentType::Step,
                looped: false,
            }],
        )
        .unwrap();
        g.set_parameters(0, 0.6, 0.2).unwrap();
        let mut original = g.clone();
        assert_eq!(g.configure(true, &[]), Err(Error::InvalidSegmentCount));
        assert_eq!(
            g.configure(
                true,
                &[Configuration {
                    kind: SegmentType::Alt,
                    looped: true
                }]
            ),
            Err(Error::UnsupportedAlt)
        );
        let mut actual = [Output::default(); 8];
        let mut expected = actual;
        g.process(&[0; 8], &mut actual);
        original.process(&[0; 8], &mut expected);
        assert_eq!(actual, expected);
    }
    #[test]
    fn slave_preserves_master_segment_and_phase() {
        let mut g = SegmentGenerator::new();
        g.configure_slave(2).unwrap();
        let mut out = [
            Output {
                value: 8.0,
                phase: 0.25,
                segment: 2,
            },
            Output {
                value: 8.0,
                phase: 0.75,
                segment: 1,
            },
        ];
        g.process(&[0; 2], &mut out);
        assert_eq!(
            out,
            [
                Output {
                    value: 0.75,
                    phase: 0.25,
                    segment: 2
                },
                Output {
                    value: 0.0,
                    phase: 0.75,
                    segment: 1
                }
            ]
        );
    }
    #[test]
    fn rounded_random_keeps_in_array_quirk_and_guards_full_capacity() {
        let mut configurations = [Configuration {
            kind: SegmentType::Step,
            looped: false,
        }; MAX_NUM_SEGMENTS];
        configurations[0].kind = SegmentType::Hold;
        for count in [6, MAX_NUM_SEGMENTS] {
            let mut g = SegmentGenerator::new();
            g.configure(true, &configurations[..count]).unwrap();
            g.set_parameters(0, 0.0, 4.5 / 7.0).unwrap();
            g.seed_rng(653637408);
            let mut output = [Output::default()];
            g.process(&[GATE_RISING | GATE_HIGH], &mut output);
            assert_eq!(g.rng_state(), u32::MAX);
            assert_eq!(output[0].segment, count.min(MAX_NUM_SEGMENTS - 1) as i32);
        }
    }
    #[test]
    fn guarded_source_lut_endpoints_remain_finite() {
        assert_eq!(frequency(2.0), LUT_ENV_FREQUENCY[4095]);
        assert_eq!(portamento(1.0), LUT_PORTAMENTO_COEFFICIENT[511]);
        for kind in [SegmentType::Ramp, SegmentType::Step, SegmentType::Hold] {
            let mut g = SegmentGenerator::new();
            g.configure(false, &[Configuration { kind, looped: true }])
                .unwrap();
            g.set_parameters(0, 2.0, 1.0).unwrap();
            let mut out = [Output::default(); 12];
            g.process(&[0; 12], &mut out);
            assert!(out
                .iter()
                .all(|v| v.value.is_finite() && v.phase.is_finite()));
        }
    }
}
