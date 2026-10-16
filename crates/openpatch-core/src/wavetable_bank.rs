//! Immutable wavetable playback data, constructed only on the control thread.
//!
//! Audio tables use Fourier-series reconstruction with progressively more
//! harmonics, sampled at eight times the source resolution. This original
//! implementation uses RustFFT rather than incorporating a module's DSP source.
//! Numerical behavior is compared with a pinned actual-class SDK oracle; see
//! scripts/wavetable_reference for the frozen acceptance and source boundary.

use std::{mem::size_of, sync::Arc};

use rustfft::{algorithm::Radix4, num_complex::Complex, Fft, FftDirection};

use crate::wavetable_file::MAX_WAVETABLE_SAMPLES;

const AUDIO_QUALITY: usize = 8;
const FFT_FIXED_ALLOWANCE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WavetableMode {
    Audio,
    Modulation,
}

/// Payload accounting for cache admission before allocating the output tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BankMemoryEstimate {
    /// Source samples plus all prepared tables, even if the source Arc is shared.
    pub(crate) retained_bytes: usize,
    /// Temporary buffers and a conservative allowance for FFT plan construction.
    /// Add to retained_bytes to estimate peak builder payload. The estimate
    /// excludes allocator bookkeeping, the incoming Arc's header, and stack use.
    pub(crate) scratch_bytes: usize,
}

/// Estimate without planning an FFT or allocating buffers. Validation here is
/// structural; `prepare` also checks that all source/prepared values are finite.
///
/// RustFFT 6.4.1 Radix4 uses an in-place scratch buffer of one transform length,
/// and its plan constructor starts a twiddle Vec with capacity twice that
/// length. The allowance here reserves four transform lengths per plan, plus
/// 64 KiB for plan objects and small butterflies, covering construction/shrink
/// temporaries as well as their final twiddle arrays. No generic planner/cache
/// is created. Revisit this bound if the FFT implementation/version changes.
pub(crate) fn estimate_memory(
    samples_len: usize,
    wave_points: usize,
    mode: WavetableMode,
) -> Result<BankMemoryEstimate, String> {
    let shape = BankShape::new(samples_len, wave_points, mode)?;
    let source_bytes = checked_mul(samples_len, size_of::<f32>())?;
    let table_bytes = checked_mul(shape.table_len, size_of::<f32>())?;
    let retained_bytes = checked_add(source_bytes, table_bytes)?;
    let fft_scratch = if mode == WavetableMode::Audio {
        // N input spectrum, M inverse spectrum, and M reusable FFT scratch.
        let buffers = checked_add(wave_points, checked_mul(shape.dense_points, 2)?)?;
        let plan_allowance = checked_mul(checked_add(wave_points, shape.dense_points)?, 4)?;
        checked_add(
            checked_mul(
                checked_add(buffers, plan_allowance)?,
                size_of::<Complex<f32>>(),
            )?,
            FFT_FIXED_ALLOWANCE,
        )?
    } else {
        0
    };
    // WaveAsset decodes a Vec and converts it into an Arc. During that
    // conversion the temporary Vec overlaps the final admitted source Arc.
    let scratch_bytes = checked_add(fft_scratch, source_bytes)?;
    Ok(BankMemoryEstimate {
        retained_bytes,
        scratch_bytes,
    })
}

#[derive(Debug)]
pub(crate) struct PreparedWaveBank {
    // Keep the entire original sequence, including an incomplete final wave.
    samples: Arc<[f32]>,
    wave_points: usize,
    wave_count: usize,
    dense_points: usize,
    levels: usize,
    // Level-major, then wave-major, then phase point. Empty for modulation.
    tables: Vec<f32>,
    // Drop payloads before releasing shared admission to another builder.
    lease: Option<crate::wave_prepare::BankLease>,
}

struct BankShape {
    wave_count: usize,
    dense_points: usize,
    levels: usize,
    table_len: usize,
}

impl BankShape {
    fn new(samples_len: usize, wave_points: usize, mode: WavetableMode) -> Result<Self, String> {
        if samples_len == 0 || samples_len > MAX_WAVETABLE_SAMPLES {
            return Err(format!(
                "Wavetable must contain 1..={MAX_WAVETABLE_SAMPLES} samples"
            ));
        }
        if wave_points < 2 || wave_points > samples_len {
            return Err("Wave length must be at least two points and fit one complete wave".into());
        }
        let wave_count = samples_len / wave_points;
        match mode {
            WavetableMode::Modulation => Ok(Self {
                wave_count,
                dense_points: wave_points,
                levels: 0,
                table_len: 0,
            }),
            WavetableMode::Audio => {
                if wave_points < 32 || !wave_points.is_power_of_two() {
                    return Err(
                        "Audio wave length must be a power of two of at least 32 points".into(),
                    );
                }
                let dense_points = checked_mul(wave_points, AUDIO_QUALITY)?;
                let levels = wave_points.trailing_zeros() as usize - 1;
                let table_len = checked_mul(checked_mul(wave_count, dense_points)?, levels)?;
                Ok(Self {
                    wave_count,
                    dense_points,
                    levels,
                    table_len,
                })
            }
        }
    }
}

fn checked_mul(a: usize, b: usize) -> Result<usize, String> {
    a.checked_mul(b)
        .ok_or_else(|| "Wavetable memory size overflow".into())
}

fn checked_add(a: usize, b: usize) -> Result<usize, String> {
    a.checked_add(b)
        .ok_or_else(|| "Wavetable memory size overflow".into())
}

fn zeroed_vec<T: Default + Clone>(length: usize) -> Result<Vec<T>, String> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(length)
        .map_err(|_| "Cannot allocate wavetable preparation buffer")?;
    result.resize(length, T::default());
    Ok(result)
}

fn complex_is_finite(value: &Complex<f32>) -> bool {
    value.re.is_finite() && value.im.is_finite()
}

impl PreparedWaveBank {
    /// Allocating, synchronous control-thread operation. Cache admission and
    /// lifetimes belong to the caller. RustFFT plan internals allocate through
    /// its own infallible API; our large buffers use fallible reservations.
    pub(crate) fn prepare(
        samples: Arc<[f32]>,
        wave_points: usize,
        mode: WavetableMode,
    ) -> Result<Self, String> {
        let estimate = estimate_memory(samples.len(), wave_points, mode)?;
        let shape = BankShape::new(samples.len(), wave_points, mode)?;
        if samples.iter().any(|x| !x.is_finite()) {
            return Err("Wavetable contains a nonfinite sample".into());
        }
        let mut bank = Self {
            samples,
            lease: None,
            wave_points,
            wave_count: shape.wave_count,
            dense_points: shape.dense_points,
            levels: shape.levels,
            tables: zeroed_vec(shape.table_len)?,
        };
        if mode == WavetableMode::Modulation {
            return Ok(bank);
        }

        // Both sizes are checked powers of two. Direct algorithms avoid the
        // generic planner's architecture-dependent collection of cached plans.
        let forward = Radix4::<f32>::new(wave_points, FftDirection::Forward);
        let inverse = Radix4::<f32>::new(shape.dense_points, FftDirection::Inverse);
        let scratch_len = forward
            .get_inplace_scratch_len()
            .max(inverse.get_inplace_scratch_len());
        if scratch_len > shape.dense_points {
            return Err("FFT scratch requirement exceeds the admitted memory estimate".into());
        }
        let mut spectrum: Vec<Complex<f32>> = zeroed_vec(wave_points)?;
        let mut dense: Vec<Complex<f32>> = zeroed_vec(shape.dense_points)?;
        let mut scratch = zeroed_vec(scratch_len)?;
        let level_stride = checked_mul(shape.wave_count, shape.dense_points)?;
        for wave in 0..shape.wave_count {
            let start = wave * wave_points;
            for (bin, &sample) in spectrum
                .iter_mut()
                .zip(&bank.samples[start..start + wave_points])
            {
                // Normalized Fourier coefficients; RustFFT's inverse is
                // unnormalized, so these can be resynthesized at any length.
                *bin = Complex::new(sample / wave_points as f32, 0.0);
            }
            forward.process_with_scratch(&mut spectrum, &mut scratch);
            if spectrum.iter().any(|x| !complex_is_finite(x)) {
                return Err("Wavetable FFT produced a nonfinite coefficient".into());
            }
            for level in 0..shape.levels {
                dense.fill(Complex::default());
                dense[0] = spectrum[0];
                // Real-FFT compatibility: the source's packed Nyquist scalar
                // stays in slot one during upsampling. In a complex transform
                // this is the dense transform's Nyquist coefficient, retained
                // at every level. Keep this observable image explicitly.
                dense[shape.dense_points / 2] = Complex::new(spectrum[wave_points / 2].re, 0.0);
                let last_harmonic = 1_usize << level;
                // Retain each positive harmonic and its negative-frequency
                // partner. The highest level stops below source Nyquist.
                for harmonic in 1..=last_harmonic {
                    dense[harmonic] = spectrum[harmonic];
                    dense[shape.dense_points - harmonic] = spectrum[wave_points - harmonic];
                }
                inverse.process_with_scratch(&mut dense, &mut scratch);
                if dense.iter().any(|x| !complex_is_finite(x)) {
                    return Err("Wavetable resynthesis produced a nonfinite sample".into());
                }
                let destination = level * level_stride + wave * shape.dense_points;
                for (output, value) in bank.tables[destination..destination + shape.dense_points]
                    .iter_mut()
                    .zip(&dense)
                {
                    *output = value.re;
                }
            }
        }
        // Record actual retained Vec capacity below, rather than treating the
        // nominal estimate as an assertion about a particular allocator.
        debug_assert!(bank.prepared_bytes() >= estimate.retained_bytes);
        Ok(bank)
    }

    pub(crate) fn set_lease(&mut self, lease: crate::wave_prepare::BankLease) {
        self.lease = Some(lease);
    }

    pub(crate) fn mode(&self) -> WavetableMode {
        if self.levels == 0 {
            WavetableMode::Modulation
        } else {
            WavetableMode::Audio
        }
    }

    pub(crate) fn wave_points(&self) -> usize {
        self.wave_points
    }

    pub(crate) fn wave_count(&self) -> usize {
        self.wave_count
    }

    /// Retained source/table payload, including Vec capacity and unused trailing
    /// source points. Shared source data is conservatively charged per bank.
    pub(crate) fn prepared_bytes(&self) -> usize {
        self.samples.len() * size_of::<f32>() + self.tables.capacity() * size_of::<f32>()
    }

    /// Normalized phase wraps periodically; normalized table position clamps to
    /// [0,1]. Audio octave selects a discrete floor/capped level. Nonfinite
    /// phase/position use zero. Positive infinite octave selects the last level
    /// (the zero-frequency limit); NaN/negative infinity select level zero.
    ///
    /// No allocation, locks, FFTs, or transcendentals. Interpolation uses the
    /// f32 order `x + (y - x) * t`. Pathological finite source magnitudes can
    /// overflow that arithmetic; in that case the returned sample is zero.
    /// Source-compatible f32 endpoint. A tiny negative phase can round to
    /// exactly one after floor wrapping. The source reads the next row at that
    /// endpoint instead of wrapping the row. Resolve its virtual packed layout
    /// safely, including zero partial-tail gaps; an actual final-buffer overrun
    /// is defined as zero here and is outside source numerical qualification.
    pub(crate) fn lookup_source(&self, phase: f32, position: f32, octave: f32) -> f32 {
        if self.levels == 0 || phase != 1.0 {
            return self.lookup(phase, position, octave);
        }
        let position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let level = if octave.is_nan() || octave <= 0.0 {
            0
        } else {
            (octave as usize).min(self.levels - 1)
        };
        let wave = position * (self.wave_count - 1) as f32;
        let first = (wave as usize).min(self.wave_count - 1);
        let fraction = wave - first as f32;
        let endpoint = |row: usize| {
            if row + 1 < self.wave_count {
                self.tables[(level * self.wave_count + row + 1) * self.dense_points]
            } else if !self.samples.len().is_multiple_of(self.wave_points) {
                0.0
            } else if level + 1 < self.levels {
                self.tables[((level + 1) * self.wave_count) * self.dense_points]
            } else {
                0.0
            }
        };
        let value = if fraction > 0.0 {
            let a = endpoint(first);
            a + (endpoint(first + 1) - a) * fraction
        } else {
            endpoint(first)
        };
        if value.is_finite() {
            value
        } else {
            0.0
        }
    }

    pub(crate) fn lookup(&self, phase: f32, position: f32, octave: f32) -> f32 {
        let phase = if phase.is_finite() {
            phase - phase.floor()
        } else {
            0.0
        };
        // For a tiny negative phase, f32 subtraction can round up to exactly 1.
        let phase = if phase >= 1.0 { 0.0 } else { phase };
        let position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let point = phase * self.dense_points as f32;
        let index = (point as usize).min(self.dense_points - 1);
        let next = if index + 1 == self.dense_points {
            0
        } else {
            index + 1
        };
        let point_fraction = point - index as f32;
        let wave = position * (self.wave_count - 1) as f32;
        let wave_index = (wave as usize).min(self.wave_count - 1);
        let wave_fraction = wave - wave_index as f32;
        let level = if self.levels == 0 || octave.is_nan() || octave <= 0.0 {
            0
        } else {
            (octave as usize).min(self.levels - 1)
        };
        let table = if self.levels == 0 {
            &*self.samples
        } else {
            &self.tables
        };
        let level_start = level * self.wave_count * self.dense_points;
        let first = level_start + wave_index * self.dense_points;
        let value = interpolate(table[first + index], table[first + next], point_fraction);
        let value = if wave_fraction > 0.0 && wave_index + 1 < self.wave_count {
            let second = first + self.dense_points;
            let other = interpolate(table[second + index], table[second + next], point_fraction);
            interpolate(value, other, wave_fraction)
        } else {
            value
        };
        if value.is_finite() {
            value
        } else {
            0.0
        }
    }
}

#[inline]
fn interpolate(x: f32, y: f32, fraction: f32) -> f32 {
    x + (y - x) * fraction
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn source(values: Vec<f32>) -> Arc<[f32]> {
        values.into()
    }

    fn near(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected}"
        );
    }

    #[test]
    fn dc_survives_every_audio_level_and_no_normalization_occurs() {
        let bank =
            PreparedWaveBank::prepare(source(vec![2.5; 32]), 32, WavetableMode::Audio).unwrap();
        for octave in [-3.0, 0.0, 1.0, 3.0, 100.0, f32::NAN, f32::INFINITY] {
            for phase in [0.0, 0.123, 0.5, 0.999, 1.0, -0.25] {
                near(bank.lookup(phase, 0.0, octave), 2.5, 1e-6);
            }
        }
        assert_eq!(bank.wave_points(), 32);
        assert_eq!(bank.wave_count(), 1);
    }

    #[test]
    fn single_harmonic_is_reconstructed_at_dense_grid_points() {
        let values = (0..64).map(|i| (TAU * i as f32 / 64.0).sin()).collect();
        let bank = PreparedWaveBank::prepare(source(values), 64, WavetableMode::Audio).unwrap();
        for point in 0..512 {
            let phase = point as f32 / 512.0;
            near(bank.lookup(phase, 0.0, 0.0), (TAU * phase).sin(), 2e-6);
        }
    }

    #[test]
    fn levels_retain_only_the_requested_harmonics_and_floor_octaves() {
        let values = (0..64)
            .map(|i| {
                let phase = TAU * i as f32 / 64.0;
                0.25 + phase.sin() + 0.5 * (phase * 3.0).sin() + 0.125 * (phase * 16.0).cos()
            })
            .collect();
        let bank = PreparedWaveBank::prepare(source(values), 64, WavetableMode::Audio).unwrap();
        for point in 0..512 {
            let phase = point as f32 / 512.0;
            let angle = TAU * phase;
            near(bank.lookup(phase, 0.0, 1.999), 0.25 + angle.sin(), 3e-6);
            near(
                bank.lookup(phase, 0.0, 2.0),
                0.25 + angle.sin() + 0.5 * (3.0 * angle).sin(),
                3e-6,
            );
            near(
                bank.lookup(phase, 0.0, 99.0),
                0.25 + angle.sin() + 0.5 * (3.0 * angle).sin() + 0.125 * (16.0 * angle).cos(),
                5e-6,
            );
        }
    }

    #[test]
    fn position_crossfades_audio_waves_and_clamps_endpoints() {
        let values = [vec![-0.5; 32], vec![1.5; 32]].concat();
        let bank = PreparedWaveBank::prepare(source(values), 32, WavetableMode::Audio).unwrap();
        for (position, expected) in [
            (-1.0, -0.5),
            (0.0, -0.5),
            (0.25, 0.0),
            (0.5, 0.5),
            (1.0, 1.5),
            (2.0, 1.5),
        ] {
            near(bank.lookup(0.43, position, 3.0), expected, 1e-6);
        }
    }

    #[test]
    fn modulation_accepts_nonpower_two_and_wraps_phase_with_linear_interpolation() {
        let bank = PreparedWaveBank::prepare(
            source(vec![0.0, 1.0, -1.0, 2.0, 3.0, 1.0]),
            3,
            WavetableMode::Modulation,
        )
        .unwrap();
        for (phase, expected) in [
            (0.0, 0.0),
            (1.0 / 6.0, 0.5),
            (1.0 / 3.0, 1.0),
            (5.0 / 6.0, -0.5),
            (1.0, 0.0),
            (-1.0 / 6.0, -0.5),
            (2.0, 0.0),
        ] {
            near(bank.lookup(phase, 0.0, 0.0), expected, 1e-6);
            near(bank.lookup(phase, 0.5, 0.0), expected + 1.0, 1e-6);
        }
        near(bank.lookup(0.0, 1.0, 0.0), 2.0, 0.0);
        assert_eq!(bank.wave_count(), 2);
    }

    #[test]
    fn trailing_partial_wave_is_retained_but_not_played() {
        for mode in [WavetableMode::Audio, WavetableMode::Modulation] {
            let bank = PreparedWaveBank::prepare(
                source([vec![0.5; 32], vec![9.0; 31]].concat()),
                32,
                mode,
            )
            .unwrap();
            assert_eq!(bank.wave_count(), 1);
            assert_eq!(bank.samples.len(), 63);
            near(bank.lookup(0.5, 1.0, 3.0), 0.5, 1e-6);
            assert_eq!(
                bank.prepared_bytes(),
                estimate_memory(63, 32, mode).unwrap().retained_bytes
            );
        }
    }

    #[test]
    fn rejects_invalid_shapes_and_nonfinite_including_unused_tail() {
        for (count, points, mode) in [
            (0, 32, WavetableMode::Audio),
            (31, 32, WavetableMode::Audio),
            (64, 16, WavetableMode::Audio),
            (64, 33, WavetableMode::Audio),
            (32, 1, WavetableMode::Modulation),
            (MAX_WAVETABLE_SAMPLES + 1, 32, WavetableMode::Audio),
            (usize::MAX, usize::MAX, WavetableMode::Audio),
        ] {
            assert!(estimate_memory(count, points, mode).is_err());
        }
        for mode in [WavetableMode::Audio, WavetableMode::Modulation] {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut samples = vec![0.0; 33];
                samples[32] = value;
                assert!(PreparedWaveBank::prepare(source(samples), 32, mode).is_err());
            }
        }
        // The filtered resynthesis of this finite square can exceed f32's
        // finite range, so preparation must reject the entire bank.
        let huge = (0..32)
            .map(|i| if i < 16 { f32::MAX } else { -f32::MAX })
            .collect();
        assert!(PreparedWaveBank::prepare(source(huge), 32, WavetableMode::Audio).is_err());
    }

    #[test]
    fn lookup_nonfinite_coordinates_and_arithmetic_are_deterministic() {
        let bank = PreparedWaveBank::prepare(source(vec![0.25, 0.5]), 2, WavetableMode::Modulation)
            .unwrap();
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(bank.lookup(value, value, value), 0.25);
        }
        let huge = PreparedWaveBank::prepare(
            source(vec![f32::MAX, -f32::MAX]),
            2,
            WavetableMode::Modulation,
        )
        .unwrap();
        assert_eq!(huge.lookup(0.25, 0.0, 0.0), 0.0);
        let values = (0..32)
            .map(|i| (TAU * 4.0 * i as f32 / 32.0).cos())
            .collect();
        let audio = PreparedWaveBank::prepare(source(values), 32, WavetableMode::Audio).unwrap();
        near(audio.lookup(0.0, 0.0, f32::INFINITY), 1.0, 1e-6);
        near(audio.lookup(0.0, 0.0, f32::NAN), 0.0, 1e-6);
        near(audio.lookup(0.0, 0.0, f32::NEG_INFINITY), 0.0, 1e-6);
    }

    #[test]
    fn memory_estimate_counts_full_source_tables_and_builder_scratch() {
        let modulation = estimate_memory(100, 3, WavetableMode::Modulation).unwrap();
        assert_eq!(modulation.retained_bytes, 400);
        assert_eq!(modulation.scratch_bytes, 400);
        let audio = estimate_memory(65, 32, WavetableMode::Audio).unwrap();
        assert_eq!(audio.retained_bytes, (65 + 4 * 2 * 256) * 4);
        assert_eq!(
            audio.scratch_bytes,
            (32 + 2 * 256 + 4 * (32 + 256)) * 8 + FFT_FIXED_ALLOWANCE + 260
        );
        let bank =
            PreparedWaveBank::prepare(source(vec![0.0; 65]), 32, WavetableMode::Audio).unwrap();
        assert_eq!(bank.prepared_bytes(), audio.retained_bytes);
    }
    #[test]
    fn source_rounded_endpoint_resolves_rows_gaps_and_final_guard_safely() {
        let bank = PreparedWaveBank::prepare(
            source([vec![0.25; 32], vec![0.75; 32]].concat()),
            32,
            WavetableMode::Audio,
        )
        .unwrap();
        near(bank.lookup_source(1.0, 0.0, 0.0), 0.75, 1e-6);
        near(bank.lookup_source(1.0, 1.0, 0.0), 0.25, 1e-6);
        assert_eq!(bank.lookup_source(1.0, 1.0, f32::INFINITY), 0.0);
        let gap = PreparedWaveBank::prepare(
            source([vec![0.25; 32], vec![0.75; 32], vec![1.0]].concat()),
            32,
            WavetableMode::Audio,
        )
        .unwrap();
        assert_eq!(gap.lookup_source(1.0, 1.0, 0.0), 0.0);
        // Ordinary normalized lookup remains periodic for other callers.
        near(bank.lookup(1.0, 0.0, 0.0), 0.25, 1e-6);
    }
    #[test]
    fn packed_nyquist_image_is_retained_when_matching_source_tables() {
        let values = (0..32)
            .map(|i| 0.2 + if i % 2 == 0 { 0.25 } else { -0.25 })
            .collect();
        let bank = PreparedWaveBank::prepare(source(values), 32, WavetableMode::Audio).unwrap();
        for level in 0..4 {
            near(bank.lookup(0.0, 0.0, level as f32), 0.45, 1e-6);
            near(bank.lookup(1.0 / 256.0, 0.0, level as f32), -0.05, 1e-6);
        }
    }
}
