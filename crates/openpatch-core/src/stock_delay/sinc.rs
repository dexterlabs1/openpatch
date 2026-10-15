// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2002-2016, Erik de Castro Lopo <erikd@mega-nerd.com>
// All rights reserved.
//
// Pure-Rust mono SRC_SINC_FASTEST adaptation of libsamplerate 0.1.9:
// https://github.com/libsndfile/libsamplerate/tree/f6730d03c3e7660bb6ecad8816f1b09c5825142a
// Adapted from src/src_sinc.c, src/common.h and src/samplerate.c.
// See docs/licenses/libsamplerate-0.1.9.txt for full notices and source hashes.

#[path = "fastest_coeffs.rs"]
mod fastest_coeffs;

use fastest_coeffs::{COEFFS, INDEX_INC};

const SHIFT_BITS: i32 = 12;
const FP_ONE: f64 = (1_i32 << SHIFT_BITS) as f64;
const INV_FP_ONE: f64 = 1.0 / FP_ONE;
const COEFF_HALF_LEN: i32 = COEFFS.len() as i32 - 2;
// Upstream uses lrint(2.5 * 2462 / 128 * SRC_MAX_RATIO), with SRC_MAX_RATIO=256.
const BUFFER_LEN: i32 = 12_310;
const MIN_RATIO: f64 = 0.25;
const MAX_RATIO: f64 = 4.0;

/// Streaming mono libsamplerate 0.1.9 sinc state, with end_of_input always false.
///
/// Construct off the audio thread. Processing, resetting and adopting state never
/// allocate. Ratios are restricted to the range used by Free Delay: 0.25..=4.0.
/// Input and output counts preserve upstream's lookahead and within-call ratio
/// interpolation, including calls that consume input without producing output.
#[derive(Debug)]
pub(super) struct MonoSinc {
    buffer: Box<[f32]>,
    b_current: i32,
    b_end: i32,
    last_position: f64,
    last_ratio: f64,
}

impl MonoSinc {
    pub(super) fn new() -> Self {
        // Upstream allocates one extra sample and fills it with 0xAA as a guard.
        let mut buffer = vec![0.0; BUFFER_LEN as usize + 1].into_boxed_slice();
        buffer[BUFFER_LEN as usize] = f32::from_bits(0xaaaa_aaaa);
        Self {
            buffer,
            b_current: 0,
            b_end: 0,
            last_position: 0.0,
            last_ratio: 0.0,
        }
    }

    pub(super) fn reset(&mut self) {
        self.b_current = 0;
        self.b_end = 0;
        self.last_position = 0.0;
        self.last_ratio = 0.0;
        self.buffer[..BUFFER_LEN as usize].fill(0.0);
        self.buffer[BUFFER_LEN as usize] = f32::from_bits(0xaaaa_aaaa);
    }

    /// Transfer the entire stream state, including its preallocated buffer.
    /// The caller owns both states exclusively; the donor receives our old state.
    #[cfg(test)]
    pub(super) fn adopt(&mut self, old: &mut Self) {
        std::mem::swap(self, old);
    }

    /// Duplicate state for the immutable offline adoption API. This copies the
    /// history into our existing allocation; real-time handoffs swap Runtime.
    pub(super) fn copy_state_from(&mut self, old: &Self) {
        self.buffer.copy_from_slice(&old.buffer);
        self.b_current = old.b_current;
        self.b_end = old.b_end;
        self.last_position = old.last_position;
        self.last_ratio = old.last_ratio;
    }

    pub(super) fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        ratio: f64,
    ) -> (usize, usize) {
        // This is outside the source's supported-call contract. Do not let a bad
        // control value corrupt indices or force an unbounded filter loop.
        if !ratio.is_finite() || !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
            return (0, 0);
        }

        // src_process() initializes last_ratio on the first call after src_reset.
        if self.last_ratio < 1.0 / 256.0 {
            self.last_ratio = ratio;
        }
        let previous_ratio = self.last_ratio;
        let mut src_ratio = previous_ratio;
        let mut in_used = 0;
        let mut out_gen = 0;

        let mut count = (COEFF_HALF_LEN as f64 + 2.0) / INDEX_INC as f64;
        let minimum_ratio = previous_ratio.min(ratio);
        if minimum_ratio < 1.0 {
            count /= minimum_ratio;
        }
        let half_filter_len = lrint(count) + 1;

        let mut input_index = self.last_position;
        let rem = fmod_one(input_index);
        self.b_current = (self.b_current + lrint(input_index - rem)) % BUFFER_LEN;
        input_index = rem;

        while out_gen < output.len() {
            let mut samples_in_hand = (self.b_end - self.b_current + BUFFER_LEN) % BUFFER_LEN;
            if samples_in_hand <= half_filter_len {
                if !self.prepare_data(input, &mut in_used, half_filter_len) {
                    break;
                }
                samples_in_hand = (self.b_end - self.b_current + BUFFER_LEN) % BUFFER_LEN;
                if samples_in_hand <= half_filter_len {
                    break;
                }
            }

            // Deliberately divide by the requested output count, not by the
            // number that this call will eventually be able to produce.
            if (previous_ratio - ratio).abs() > 1e-10 {
                src_ratio = previous_ratio
                    + out_gen as f64 * (ratio - previous_ratio) / output.len() as f64;
            }
            let float_increment = INDEX_INC as f64 * src_ratio.min(1.0);
            let increment = double_to_fp(float_increment);
            let start_filter_index = double_to_fp(input_index * float_increment);
            output[out_gen] = ((float_increment / INDEX_INC as f64)
                * self.calc_output_single(increment, start_filter_index))
                as f32;
            out_gen += 1;

            input_index += 1.0 / src_ratio;
            let rem = fmod_one(input_index);
            self.b_current = (self.b_current + lrint(input_index - rem)) % BUFFER_LEN;
            input_index = rem;
        }

        self.last_position = input_index;
        // The source retains the last ratio actually used, even on short calls.
        self.last_ratio = src_ratio;
        (in_used, out_gen)
    }

    /// Source prepare_data(), specialized to one channel and no end-of-input.
    fn prepare_data(&mut self, input: &[f32], in_used: &mut usize, half_filter_len: i32) -> bool {
        let available;
        if self.b_current == 0 {
            available = BUFFER_LEN - 2 * half_filter_len;
            self.b_current = half_filter_len;
            self.b_end = half_filter_len;
        } else if self.b_end + half_filter_len + 1 < BUFFER_LEN {
            available = (BUFFER_LEN - self.b_current - half_filter_len).max(0);
        } else {
            let len = self.b_end - self.b_current;
            let start = self.b_current - half_filter_len;
            let copy_len = half_filter_len + len;
            // The valid ratio range keeps these indices inside the allocation.
            if start < 0 || copy_len < 0 || start + copy_len > BUFFER_LEN {
                return false;
            }
            self.buffer
                .copy_within(start as usize..(start + copy_len) as usize, 0);
            self.b_current = half_filter_len;
            self.b_end = self.b_current + len;
            available = (BUFFER_LEN - self.b_current - half_filter_len).max(0);
        }

        let len = (input.len() - *in_used).min(available as usize);
        if self.b_end < 0 || self.b_end as usize + len > BUFFER_LEN as usize {
            return false;
        }
        let end = self.b_end as usize;
        self.buffer[end..end + len].copy_from_slice(&input[*in_used..*in_used + len]);
        self.b_end += len as i32;
        *in_used += len;
        true
    }

    fn calc_output_single(&self, increment: i32, start_filter_index: i32) -> f64 {
        let max_filter_index = COEFF_HALF_LEN << SHIFT_BITS;
        let mut filter_index = start_filter_index;
        let coeff_count = (max_filter_index - filter_index) / increment;
        filter_index += coeff_count * increment;
        let mut data_index = self.b_current - coeff_count;

        let mut left = 0.0;
        loop {
            let icoeff = interpolated_coefficient(filter_index);
            left += icoeff * self.history_sample(data_index) as f64;
            filter_index -= increment;
            data_index += 1;
            if filter_index < 0 {
                break;
            }
        }

        filter_index = increment - start_filter_index;
        let coeff_count = (max_filter_index - filter_index) / increment;
        filter_index += coeff_count * increment;
        data_index = self.b_current + 1 + coeff_count;
        let mut right = 0.0;
        loop {
            let icoeff = interpolated_coefficient(filter_index);
            right += icoeff * self.history_sample(data_index) as f64;
            filter_index -= increment;
            data_index -= 1;
            if filter_index <= 0 {
                break;
            }
        }
        left + right
    }

    fn history_sample(&self, index: i32) -> f32 {
        // The C algorithm can request history before its allocation if the ratio
        // drops steeply during startup. Such a C read is undefined. Extend the
        // initial zero history for that case; all defined source reads use the
        // original stored sample, with unchanged filter arithmetic and counts.
        self.buffer.get(index as usize).copied().unwrap_or(0.0)
    }
}

fn interpolated_coefficient(filter_index: i32) -> f64 {
    let fraction = (filter_index & ((1 << SHIFT_BITS) - 1)) as f64 * INV_FP_ONE;
    let index = (filter_index >> SHIFT_BITS) as usize;
    // In C both operands of this subtraction are coeff_t (float), so round the
    // difference to f32 BEFORE promoting it for multiplication by the fraction.
    let difference = (COEFFS[index + 1] - COEFFS[index]) as f64;
    COEFFS[index] as f64 + fraction * difference
}

fn double_to_fp(value: f64) -> i32 {
    lrint(value * FP_ONE)
}

fn lrint(value: f64) -> i32 {
    // C lrint with its normal FE_TONEAREST rounding mode, including exact ties.
    value.round_ties_even() as i32
}

fn fmod_one(value: f64) -> f64 {
    let result = value - lrint(value) as f64;
    if result < 0.0 {
        result + 1.0
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_requires_future_input_and_keeps_output_tail_untouched() {
        let mut src = MonoSinc::new();
        let mut output = [123.0; 16];
        assert_eq!(src.process(&[0.0; 16], &mut output, 1.0), (16, 0));
        assert_eq!(output, [123.0; 16]);
        assert_eq!(src.process(&[0.0; 16], &mut output, 1.0), (16, 12));
        assert_eq!(&output[..12], &[0.0; 12]);
        assert_eq!(&output[12..], &[123.0; 4]);
    }

    #[test]
    fn empty_output_does_not_consume_input() {
        let mut src = MonoSinc::new();
        assert_eq!(src.process(&[1.0; 16], &mut [], 0.25), (0, 0));
        assert_eq!(src.last_ratio, 0.25);
        assert_eq!(src.b_current, 0);
    }

    #[test]
    fn reset_replays_startup_and_ratio_initialization() {
        let mut src = MonoSinc::new();
        let mut fresh = MonoSinc::new();
        let mut a = [0.0; 16];
        let mut b = [0.0; 16];
        for _ in 0..40 {
            src.process(&[0.75; 16], &mut a, 0.5);
        }
        src.reset();
        for call in 0..20 {
            let ratio = if call < 10 { 0.25 } else { 4.0 };
            let input = [call as f32 * 0.1; 16];
            a.fill(123.0);
            b.fill(123.0);
            assert_eq!(
                src.process(&input, &mut a, ratio),
                fresh.process(&input, &mut b, ratio)
            );
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
    }

    #[test]
    fn adoption_transfers_partial_history_and_ratio_ramp() {
        let mut source = MonoSinc::new();
        let mut adopted = MonoSinc::new();
        let mut control = MonoSinc::new();
        let mut a = [0.0; 16];
        let mut b = [0.0; 16];
        for call in 0..18 {
            let ratio = if call < 7 { 0.5 } else { 2.0 };
            let input = [(call as f32 * 0.7).sin(); 16];
            assert_eq!(
                source.process(&input, &mut a, ratio),
                control.process(&input, &mut b, ratio)
            );
        }
        let buffer_pointer = source.buffer.as_ptr();
        adopted.adopt(&mut source);
        assert_eq!(adopted.buffer.as_ptr(), buffer_pointer);
        for call in 0..24 {
            let ratio = if call < 9 { 4.0 } else { 0.25 };
            let input = [(call as f32 * 0.2).cos(); 16];
            assert_eq!(
                adopted.process(&input, &mut a, ratio),
                control.process(&input, &mut b, ratio)
            );
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
    }

    #[test]
    fn block_boundaries_preserve_stream_across_buffer_compaction() {
        let input: Vec<f32> = (0..28_000)
            .map(|index| ((index as f32 * 0.013).sin() + (index as f32 * 0.037).cos()) * 0.2)
            .collect();
        let collect = |chunk_size| {
            let mut src = MonoSinc::new();
            let buffer_pointer = src.buffer.as_ptr();
            let mut result = Vec::new();
            let mut output = [0.0; 16];
            for chunk in input.chunks(chunk_size) {
                let (used, generated) = src.process(chunk, &mut output, 1.0);
                assert_eq!(used, chunk.len());
                assert_eq!(src.buffer.as_ptr(), buffer_pointer);
                result.extend(output[..generated].iter().map(|value| value.to_bits()));
            }
            // Constant-ratio mono sinc retains twenty frames of future input.
            assert_eq!(result.len(), input.len() - 20);
            result
        };
        assert_eq!(collect(16), collect(8));
    }

    #[test]
    fn steep_startup_ratio_drop_extends_zero_history_safely() {
        let mut src = MonoSinc::new();
        let mut output = [123.0; 16];
        for call in 0..8 {
            let ratio = if call == 0 { 1.0 } else { 0.25 };
            let (_, generated) = src.process(&[0.0; 16], &mut output, ratio);
            assert_eq!(&output[..generated], vec![0.0; generated].as_slice());
            if call == 4 {
                assert_eq!(generated, 2);
                assert_eq!(src.b_current, 22);
                assert_eq!(src.last_ratio, 61.0 / 64.0);
            }
        }
    }

    #[test]
    fn rounding_uses_nearest_even() {
        assert_eq!(
            [lrint(0.5), lrint(1.5), lrint(2.5), lrint(3.5)],
            [0, 2, 2, 4]
        );
        assert_eq!(double_to_fp(0.5 / FP_ONE), 0);
        assert_eq!(double_to_fp(1.5 / FP_ONE), 2);
    }
}
