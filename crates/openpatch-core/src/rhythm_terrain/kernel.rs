// Copyright 2011 Emilie Gillet. GPL-3.0-or-later.
// Source-preserving per-instance adaptation of Grids pattern_generator.{h,cc}
// and clock.{h,cc}, eurorack f4cfa7697095a28d292bff758beb51deb49b5a86.
// The avrlib 276b2887 RNG is an explicit, owned 16-bit Galois LFSR.

use super::resources::{DRUM_MAP, EUCLIDEAN, NODES, TEMPO_PHASE_INCREMENT};

pub const OUTPUT_MODE_EUCLIDEAN: u8 = 0;
pub const OUTPUT_MODE_DRUMS: u8 = 1;
pub const CLOCK_RESOLUTION_4_PPQN: u8 = 0;
pub const CLOCK_RESOLUTION_8_PPQN: u8 = 1;
pub const CLOCK_RESOLUTION_24_PPQN: u8 = 2;
pub const PULSES_PER_STEP: u8 = 3;
pub const STEPS_PER_PATTERN: u8 = 32;
pub const PULSE_DURATION: u8 = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Drums: x, y, randomness. Euclidean: three lengths, each encoded 0..255.
    pub options: [u8; 3],
    pub density: [u8; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub clock_resolution: u8,
    pub output_mode: u8,
    pub output_clock: bool,
    pub tap_tempo: bool,
    pub gate_mode: bool,
    pub swing: bool,
}

impl Options {
    pub fn pack(self) -> u8 {
        self.clock_resolution
            | if self.swing { 0 } else { 0x08 }
            | if self.tap_tempo { 0x10 } else { 0 }
            | if self.output_clock { 0x20 } else { 0 }
            | if self.output_mode == OUTPUT_MODE_DRUMS {
                0x40
            } else {
                0
            }
            | if self.gate_mode { 0 } else { 0x80 }
    }

    /// Guarded extension for malformed EEPROM bytes: low bits 4..7 are
    /// undefined enum loads in the original C++, and clamp safely here.
    /// Source-defined bytes use low bits 0..3; native controls use 0..2.
    pub fn unpack(byte: u8) -> Self {
        Self {
            clock_resolution: (byte & 7).min(CLOCK_RESOLUTION_24_PPQN),
            output_mode: u8::from(byte & 0x40 != 0),
            output_clock: byte & 0x20 != 0,
            tap_tempo: byte & 0x10 != 0,
            gate_mode: byte & 0x80 == 0,
            swing: byte & 8 == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternSnapshot {
    pub state: u8,
    pub pulse: u8,
    pub step: u8,
    pub euclidean_step: [u8; 3],
    pub part_perturbation: [u8; 3],
    pub pulse_duration_counter: u8,
    pub first_beat: bool,
    pub beat: bool,
    pub rng_state: u16,
    pub swing_amount: i8,
}

#[derive(Clone, Debug)]
pub struct PatternGenerator {
    pub settings: Settings,
    pub options: Options,
    pulse: u8,
    step: u8,
    euclidean_step: [u8; 3],
    first_beat: bool,
    beat: bool,
    state: u8,
    part_perturbation: [u8; 3],
    pulse_duration_counter: u8,
    rng_state: u16,
}

impl PatternGenerator {
    /// Reproduce source cold static storage, with an explicit owned RNG seed.
    /// The source seed is 16-bit; zero remains the source's absorbing state.
    pub fn new(seed: u32) -> Self {
        Self {
            settings: Settings::default(),
            options: Options::default(),
            pulse: 0,
            step: 0,
            euclidean_step: [0; 3],
            first_beat: false,
            beat: false,
            state: 0,
            part_perturbation: [0; 3],
            pulse_duration_counter: 0,
            rng_state: seed as u16,
        }
    }

    pub fn reset(&mut self) {
        self.step = 0;
        self.pulse = 0;
        self.euclidean_step = [0; 3];
    }

    pub fn retrigger(&mut self) {
        self.evaluate();
    }

    pub fn tick_clock(&mut self, num_pulses: u8) {
        self.evaluate();
        self.beat = self.step & 7 == 0;
        self.first_beat = self.step == 0;
        self.pulse = self.pulse.wrapping_add(num_pulses);
        while self.pulse >= PULSES_PER_STEP {
            self.pulse -= PULSES_PER_STEP;
            if self.step & 1 == 0 {
                for value in &mut self.euclidean_step {
                    *value = value.wrapping_add(1);
                }
            }
            self.step = self.step.wrapping_add(1);
        }
        if self.step >= STEPS_PER_PATTERN {
            self.step -= STEPS_PER_PATTERN;
        }
    }

    pub fn increment_pulse_counter(&mut self) {
        self.pulse_duration_counter = self.pulse_duration_counter.wrapping_add(1);
        if self.pulse_duration_counter >= PULSE_DURATION && !self.options.gate_mode {
            self.state = 0;
        }
    }

    pub fn clock_falling_edge(&mut self) {
        if self.options.gate_mode {
            self.state = 0;
        }
    }

    pub fn state(&self) -> u8 {
        self.state
    }
    pub fn step(&self) -> u8 {
        self.step
    }
    pub fn on_beat(&self) -> bool {
        self.beat
    }
    pub fn on_first_beat(&self) -> bool {
        self.first_beat
    }
    pub fn swing(&self) -> bool {
        self.options.swing
    }
    pub fn output_clock(&self) -> bool {
        self.options.output_clock
    }
    pub fn tap_tempo(&self) -> bool {
        self.options.tap_tempo
    }
    pub fn gate_mode(&self) -> bool {
        self.options.gate_mode
    }
    pub fn output_mode(&self) -> u8 {
        self.options.output_mode
    }
    pub fn clock_resolution(&self) -> u8 {
        self.options.clock_resolution
    }
    pub fn set_swing(&mut self, value: u8) {
        self.options.swing = value != 0;
    }
    pub fn set_output_clock(&mut self, value: u8) {
        self.options.output_clock = value != 0;
    }
    pub fn set_tap_tempo(&mut self, value: u8) {
        self.options.tap_tempo = value != 0;
    }
    pub fn set_output_mode(&mut self, value: u8) {
        self.options.output_mode = value;
    }
    pub fn set_clock_resolution(&mut self, value: u8) {
        self.options.clock_resolution = value.min(2);
    }
    pub fn set_gate_mode(&mut self, value: bool) {
        self.options.gate_mode = value;
    }
    pub fn mutable_settings(&mut self) -> &mut Settings {
        &mut self.settings
    }
    pub fn rng_state(&self) -> u16 {
        self.rng_state
    }
    pub fn seed(&mut self, seed: u32) {
        self.rng_state = seed as u16;
    }
    pub fn led_pattern(&self) -> u8 {
        ((self.state & 1) << 3) | ((self.state & 2) << 1) | ((self.state & 4) >> 1)
    }

    pub fn swing_amount(&self) -> i8 {
        if self.options.swing && self.options.output_mode == OUTPUT_MODE_DRUMS {
            let value = mul_shift8(self.settings.options[2], 43) as i8;
            if self.step & 2 == 0 {
                value
            } else {
                -value
            }
        } else {
            0
        }
    }

    pub fn snapshot(&self) -> PatternSnapshot {
        PatternSnapshot {
            state: self.state,
            pulse: self.pulse,
            step: self.step,
            euclidean_step: self.euclidean_step,
            part_perturbation: self.part_perturbation,
            pulse_duration_counter: self.pulse_duration_counter,
            first_beat: self.first_beat,
            beat: self.beat,
            rng_state: self.rng_state,
            swing_amount: self.swing_amount(),
        }
    }

    fn random_update(&mut self) {
        self.rng_state = (self.rng_state >> 1) ^ (0u16.wrapping_sub(self.rng_state & 1) & 0xb400);
    }
    fn random_byte(&mut self) -> u8 {
        self.random_update();
        (self.rng_state >> 8) as u8
    }

    fn evaluate(&mut self) {
        self.state = 0;
        self.pulse_duration_counter = 0;
        self.random_update();
        self.state |= 0x40;
        self.state |= self.rng_state as u8 & 0x80;
        if self.output_clock() {
            self.state |= 0x10;
        }
        if self.pulse != 0 {
            return;
        }
        if self.options.output_mode == OUTPUT_MODE_EUCLIDEAN {
            self.evaluate_euclidean();
        } else {
            self.evaluate_drums();
        }
    }

    fn evaluate_drums(&mut self) {
        if self.step == 0 {
            for i in 0..3 {
                let randomness = if self.options.swing {
                    0
                } else {
                    self.settings.options[2] >> 2
                };
                let random = self.random_byte();
                self.part_perturbation[i] = mul_shift8(random, randomness);
            }
        }
        let mut accent_bits = 0;
        for i in 0..3 {
            let level = Self::read_drum_map(
                self.step,
                i as u8,
                self.settings.options[0],
                self.settings.options[1],
            );
            let level = if level < 255 - self.part_perturbation[i] {
                level + self.part_perturbation[i]
            } else {
                255
            };
            let instrument_mask = 1 << i;
            if level > !self.settings.density[i] {
                if level > 192 {
                    accent_bits |= instrument_mask;
                }
                self.state |= instrument_mask;
            }
        }
        if self.output_clock() {
            self.state |= if accent_bits != 0 { 0x08 } else { 0 };
            self.state |= if self.step == 0 { 0x20 } else { 0 };
        } else {
            self.state |= accent_bits << 3;
        }
    }

    fn evaluate_euclidean(&mut self) {
        if self.step & 1 != 0 {
            return;
        }
        let mut reset_bits = 0;
        for i in 0..3 {
            let length = (self.settings.options[i] >> 3) + 1;
            let density = self.settings.density[i] >> 3;
            let address = usize::from(length - 1) * 32 + usize::from(density);
            while self.euclidean_step[i] >= length {
                self.euclidean_step[i] -= length;
            }
            let step_mask = 1u32 << self.euclidean_step[i];
            let mask = 1 << i;
            if EUCLIDEAN[address] & step_mask != 0 {
                self.state |= mask;
            }
            if self.euclidean_step[i] == 0 {
                reset_bits |= mask;
            }
        }
        if self.output_clock() {
            self.state |= if reset_bits != 0 { 0x08 } else { 0 };
            self.state |= if reset_bits == 7 { 0x20 } else { 0 };
        } else {
            self.state |= reset_bits << 3;
        }
    }

    /// Exact double U8Mix, including its 255-weight division by 256.
    /// A source-valid drum step is 0..31 and instrument is 0..2.
    pub fn read_drum_map(step: u8, instrument: u8, x: u8, y: u8) -> u8 {
        let i = usize::from(x >> 6);
        let j = usize::from(y >> 6);
        let offset = usize::from(instrument) * 32 + usize::from(step);
        let a = NODES[DRUM_MAP[i][j]][offset];
        let b = NODES[DRUM_MAP[i + 1][j]][offset];
        let c = NODES[DRUM_MAP[i][j + 1]][offset];
        let d = NODES[DRUM_MAP[i + 1][j + 1]][offset];
        mix(
            mix(a, b, x.wrapping_shl(2)),
            mix(c, d, x.wrapping_shl(2)),
            y.wrapping_shl(2),
        )
    }
}

impl Default for PatternGenerator {
    fn default() -> Self {
        Self::new(0x21)
    }
}
fn mul_shift8(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b)) >> 8) as u8
}
fn mix(a: u8, b: u8, balance: u8) -> u8 {
    ((u16::from(a) * u16::from(255 - balance) + u16::from(b) * u16::from(balance)) >> 8) as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSnapshot {
    pub locked: bool,
    pub bpm: u16,
    pub phase: u32,
    pub phase_increment: u32,
    pub falling_edge: u8,
    pub raising_edge: bool,
    pub past_falling_edge: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Clock {
    locked: bool,
    bpm: u16,
    phase: u32,
    phase_increment: u32,
    falling_edge: u8,
}

impl Clock {
    pub fn new() -> Self {
        let mut clock = Self::default();
        clock.init();
        clock
    }
    pub fn init(&mut self) {
        self.update(120, CLOCK_RESOLUTION_24_PPQN);
        self.locked = false;
    }
    /// Source LUT domain is BPM 0..511. Out-of-domain source reads are undefined.
    pub fn update(&mut self, bpm: u16, resolution: u8) {
        self.bpm = bpm;
        self.phase_increment = TEMPO_PHASE_INCREMENT[usize::from(bpm)];
        if resolution == CLOCK_RESOLUTION_4_PPQN {
            self.phase_increment >>= 1;
        } else if resolution == CLOCK_RESOLUTION_24_PPQN {
            self.phase_increment = self.phase_increment.wrapping_mul(3);
        }
    }
    pub fn reset(&mut self) {
        self.phase = 0;
    }
    pub fn tick(&mut self) {
        self.phase = self.phase.wrapping_add(self.phase_increment);
    }
    pub fn wrap(&mut self, amount: i8) {
        if amount == 0 {
            self.phase &= 0x7fff_ffff;
            self.falling_edge = 0x40;
        } else {
            let boundary = (128i16 + i16::from(amount)) as u8;
            if (self.phase >> 24) as u8 >= boundary {
                self.phase &= 0x00ff_ffff;
            }
            self.falling_edge = boundary >> 1;
        }
    }
    pub fn raising_edge(&self) -> bool {
        self.phase < self.phase_increment
    }
    pub fn past_falling_edge(&self) -> bool {
        (self.phase >> 24) as u8 >= self.falling_edge
    }
    pub fn lock(&mut self) {
        self.locked = true;
    }
    pub fn unlock(&mut self) {
        self.locked = false;
    }
    pub fn locked(&self) -> bool {
        self.locked
    }
    pub fn bpm(&self) -> u16 {
        self.bpm
    }
    pub fn snapshot(&self) -> ClockSnapshot {
        ClockSnapshot {
            locked: self.locked,
            bpm: self.bpm,
            phase: self.phase,
            phase_increment: self.phase_increment,
            falling_edge: self.falling_edge,
            raising_edge: self.raising_edge(),
            past_falling_edge: self.past_falling_edge(),
        }
    }
}
