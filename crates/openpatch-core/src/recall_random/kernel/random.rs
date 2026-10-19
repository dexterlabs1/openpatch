// Copyright 2015 Emilie Gillet. MIT license; see crate LICENSE-THIRD-PARTY.
// Source-derived Marbles random_generator.h/random_stream.h/random_sequence.h.
// Pointer members are represented by indices so derived Clone rebases them.

const MAX_UINT32: f32 = 4294967296.0;

/// Native extension: source float-to-u32 conversion is undefined at 2^32
/// (which a rounded GetFloat can produce). Saturate only that endpoint.
fn uniform_to_word(value: f32) -> u32 {
    let scaled = value * MAX_UINT32;
    if scaled >= MAX_UINT32 {
        u32::MAX
    } else {
        scaled as u32
    }
}

#[derive(Clone, Debug)]
pub struct RandomStream {
    state: u32,
    buffer: [u32; 128],
    read: usize,
    write: usize,
}
impl RandomStream {
    pub fn new(seed: u32) -> Self {
        Self {
            state: seed,
            buffer: [0; 128],
            read: 0,
            write: 0,
        }
    }
    pub fn state(&self) -> u32 {
        self.state
    }
    pub fn get_word(&mut self) -> u32 {
        if self.read != self.write {
            let result = self.buffer[self.read];
            self.read = (self.read + 1) & 127;
            result
        } else {
            self.state = self.state.wrapping_mul(1664525).wrapping_add(1013904223);
            self.state
        }
    }
    pub fn get_float(&mut self) -> f32 {
        self.get_word() as f32 / MAX_UINT32
    }
    pub fn write(&mut self, word: u32) {
        let next = (self.write + 1) & 127;
        if next != self.read {
            self.buffer[self.write] = word;
            self.write = next;
        }
        // Upstream RandomGenerator::Mix is intentionally a no-op.
    }
}

#[derive(Clone, Debug)]
pub struct RandomSequence {
    loop_values: [f32; 16],
    history: [f32; 16],
    loop_write_head: i32,
    length: i32,
    step: i32,
    record_head: i32,
    replay_head: i32,
    replay_start: i32,
    replay_hash: u32,
    replay_shift: u32,
    deja_vu: f32,
    redo_read: usize,
    redo_write: Option<usize>,
    redo_write_history: Option<usize>,
}
impl RandomSequence {
    pub fn new(stream: &mut RandomStream) -> Self {
        let mut loop_values = [0.0; 16];
        for value in &mut loop_values {
            *value = stream.get_float();
        }
        Self {
            loop_values,
            history: [0.0; 16],
            loop_write_head: 0,
            length: 8,
            step: 0,
            record_head: 0,
            replay_head: -1,
            replay_start: 0,
            replay_hash: 0,
            replay_shift: 0,
            deja_vu: 0.0,
            redo_read: 0,
            redo_write: None,
            redo_write_history: None,
        }
    }
    pub fn record(&mut self) {
        self.replay_start = self.record_head;
        self.replay_head = -1;
    }
    pub fn replay_pseudo_random(&mut self, hash: u32) {
        self.replay_head = self.replay_start;
        self.replay_hash = hash;
        self.replay_shift = 0;
    }
    pub fn replay_shifted(&mut self, shift: u32) {
        self.replay_head = self.replay_start;
        self.replay_hash = 0;
        self.replay_shift = shift;
    }
    fn get_replay_value(&self) -> f32 {
        // C++ unsigned arithmetic from replay_shift_ is preserved.
        let h = (self.replay_head as u32)
            .wrapping_sub(1)
            .wrapping_sub(self.replay_shift)
            .wrapping_add(32)
            % 16;
        if self.replay_hash == 0 {
            self.history[h as usize]
        } else {
            let word = uniform_to_word(self.history[h as usize]);
            let word = (word ^ self.replay_hash)
                .wrapping_mul(1664525)
                .wrapping_add(1013904223);
            word as f32 / MAX_UINT32
        }
    }
    pub fn rewrite_value(&mut self, value: f32) -> f32 {
        if self.replay_head >= 0 {
            return self.get_replay_value();
        }
        if let Some(i) = self.redo_write {
            self.loop_values[i] = 1.0 + value;
        }
        let mut result = self.loop_values[self.redo_read];
        if result >= 1.0 {
            result -= 1.0;
        } else {
            result = 0.5;
        }
        if let Some(i) = self.redo_write_history {
            self.history[i] = result;
        }
        result
    }
    pub fn next_value(
        &mut self,
        deterministic: bool,
        value: f32,
        stream: &mut RandomStream,
    ) -> f32 {
        if self.replay_head >= 0 {
            self.replay_head = (self.replay_head + 1) % 16;
            return self.get_replay_value();
        }
        let p_sqrt = 2.0 * self.deja_vu - 1.0;
        let p = p_sqrt * p_sqrt;
        let mutate = stream.get_float() < p;
        if mutate && self.deja_vu <= 0.5 {
            self.redo_write = Some(self.loop_write_head as usize);
            self.loop_values[self.loop_write_head as usize] = if deterministic {
                1.0 + value
            } else {
                stream.get_float()
            };
            self.loop_write_head = (self.loop_write_head + 1) % 16;
            self.step = self.length - 1;
        } else {
            self.redo_write = None;
            if mutate {
                self.step = (stream.get_float() * self.length as f32) as i32;
            } else {
                self.step += 1;
                if self.step >= self.length {
                    self.step = 0;
                }
            }
        }
        let i = self.loop_write_head + 16 - self.length + self.step;
        self.redo_read = (i % 16) as usize;
        let mut result = self.loop_values[self.redo_read];
        if result >= 1.0 {
            result -= 1.0;
        } else if deterministic {
            result = 0.5;
        }
        self.redo_write_history = Some(self.record_head as usize);
        self.history[self.record_head as usize] = result;
        self.record_head = (self.record_head + 1) % 16;
        result
    }
    pub fn next_vector(&mut self, destination: &mut [f32], stream: &mut RandomStream) {
        let seed = self.next_value(false, 0.0, stream);
        let mut word = uniform_to_word(seed);
        for value in destination {
            *value = word as f32 / MAX_UINT32;
            word = word.wrapping_mul(1664525).wrapping_add(1013904223);
        }
    }
    pub fn set_deja_vu(&mut self, value: f32) {
        self.deja_vu = value;
    }
    pub fn set_length(&mut self, length: i32) {
        if !(1..=16).contains(&length) {
            return;
        }
        self.length = length;
        self.step %= length;
    }
    pub fn deja_vu(&self) -> f32 {
        self.deja_vu
    }
    pub fn length(&self) -> i32 {
        self.length
    }
    pub fn reset(&mut self) {
        self.step = self.length - 1;
    }
}
