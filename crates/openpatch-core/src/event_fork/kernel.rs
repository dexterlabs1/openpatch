// Copyright 2012 Emilie Gillet; Rust adaptation Copyright 2026 OpenPatch contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
//! Raw Branches input/decision/output/LED/PRNG scan, adapted from branches.cc.
//! Pinned source: pichenettes/eurorack f4cfa7697095a28d292bff758beb51deb49b5a86.
//! One call is one firmware while-loop scan. ADC/button scheduling, EEPROM,
//! electrical input inversion and CPU/timer wall-clock timing belong to the host.

/// The original ADC calibration table, including both duplicated endpoints.
pub const LINEAR_TABLE: [u16; 256] = [
    0, 0, 259, 518, 777, 1036, 1295, 1554, 1813, 2072, 2331, 2590, 2849, 3108, 3367, 3626, 3885,
    4145, 4404, 4663, 4922, 5181, 5440, 5699, 5958, 6217, 6476, 6735, 6994, 7253, 7512, 7771, 8030,
    8289, 8548, 8807, 9066, 9325, 9584, 9843, 10102, 10361, 10620, 10879, 11138, 11397, 11656,
    11915, 12174, 12434, 12693, 12952, 13211, 13470, 13729, 13988, 14247, 14506, 14765, 15024,
    15283, 15542, 15801, 16060, 16319, 16578, 16837, 17096, 17355, 17614, 17873, 18132, 18391,
    18650, 18909, 19168, 19427, 19686, 19945, 20204, 20463, 20723, 20982, 21241, 21500, 21759,
    22018, 22277, 22536, 22795, 23054, 23313, 23572, 23831, 24090, 24349, 24608, 24867, 25126,
    25385, 25644, 25903, 26162, 26421, 26680, 26939, 27198, 27457, 27716, 27975, 28234, 28493,
    28753, 29012, 29271, 29530, 29789, 30048, 30307, 30566, 30825, 31084, 31343, 31602, 31861,
    32120, 32379, 32638, 32897, 33156, 33415, 33674, 33933, 34192, 34451, 34710, 34969, 35228,
    35487, 35746, 36005, 36264, 36523, 36782, 37042, 37301, 37560, 37819, 38078, 38337, 38596,
    38855, 39114, 39373, 39632, 39891, 40150, 40409, 40668, 40927, 41186, 41445, 41704, 41963,
    42222, 42481, 42740, 42999, 43258, 43517, 43776, 44035, 44294, 44553, 44812, 45072, 45331,
    45590, 45849, 46108, 46367, 46626, 46885, 47144, 47403, 47662, 47921, 48180, 48439, 48698,
    48957, 49216, 49475, 49734, 49993, 50252, 50511, 50770, 51029, 51288, 51547, 51806, 52065,
    52324, 52583, 52842, 53101, 53361, 53620, 53879, 54138, 54397, 54656, 54915, 55174, 55433,
    55692, 55951, 56210, 56469, 56728, 56987, 57246, 57505, 57764, 58023, 58282, 58541, 58800,
    59059, 59318, 59577, 59836, 60095, 60354, 60613, 60872, 61131, 61390, 61650, 61909, 62168,
    62427, 62686, 62945, 63204, 63463, 63722, 63981, 64240, 64499, 64758, 65017, 65276, 65535,
    65535,
];

#[inline]
pub const fn adc_threshold(adc: u8) -> u16 {
    LINEAR_TABLE[adc as usize]
}

/// State after the scan, including the LFSR advance at the end of the loop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    /// GPIO levels in channel order, each `[A, B]`.
    pub outputs: [[bool; 2]; 2],
    /// Last decision, retained after the gate falls.
    pub previous_state: [bool; 2],
    pub input_state: [bool; 2],
    /// Firmware values: 0 off, 1 red (B), 2 green (A).
    pub led_state: [u8; 2],
    pub led_gate_delay: [u16; 2],
    pub rng_state: u32,
}

#[derive(Clone, Debug)]
pub struct Kernel {
    state: Snapshot,
}

impl Kernel {
    /// Raw injected seed. In particular zero is preserved and is an absorbing
    /// LFSR state; source firmware itself boots with seed 1.
    pub const fn new(seed: u32) -> Self {
        Self {
            state: Snapshot {
                outputs: [[false; 2]; 2],
                previous_state: [false; 2],
                input_state: [false; 2],
                led_state: [0; 2],
                led_gate_delay: [0; 2],
                rng_state: seed,
            },
        }
    }

    pub const fn snapshot(&self) -> Snapshot {
        self.state
    }

    pub fn process(
        &mut self,
        gates: [bool; 2],
        thresholds: [u16; 2],
        toggle: [bool; 2],
        latch: [bool; 2],
    ) -> Snapshot {
        let mut random_words = self.state.rng_state;
        for i in 0..2 {
            let new_input_state = gates[i];
            if new_input_state || latch[i] {
                self.state.led_gate_delay[i] = 0x1ff;
            }
            if new_input_state && !self.state.input_state[i] {
                let random = random_words as u16;
                let threshold = thresholds[i];
                let mut outcome = random >= threshold && threshold != 65535;
                if toggle[i] {
                    outcome ^= self.state.previous_state[i];
                }
                self.state.previous_state[i] = outcome;
                self.state.outputs[i] = [!outcome, outcome];
                self.state.led_state[i] = if outcome { 1 } else { 2 };
            } else if !new_input_state && self.state.input_state[i] && !latch[i] {
                self.state.outputs[i] = [false; 2];
            }
            self.state.input_state[i] = new_input_state;
            random_words >>= 16;
            if self.state.led_gate_delay[i] != 0 {
                self.state.led_gate_delay[i] -= 1;
                if self.state.led_gate_delay[i] == 0 {
                    self.state.led_state[i] = 0;
                }
            }
        }
        let rng = self.state.rng_state;
        self.state.rng_state = (rng >> 1) ^ (0_u32.wrapping_sub(rng & 1) & 0xd000_0001);
        self.state
    }
}
