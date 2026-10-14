//! Original inverse-A approximation for the gray noise generator.
//!
//! A bounded, control-thread-only fit avoids long FIR convolutions and FFT work
//! bursts in the audio callback. Twelve digital peaking sections fit the standard
//! inverse-A magnitude; four Butterworth sections supply soft band edges. This
//! is intentionally not the source plugin's block-FFT implementation.
//!
//! Biquad mathematics: W3C Audio EQ Cookbook (2021),
//! https://www.w3.org/TR/audio-eq-cookbook/ . The fitting strategy, frequency
//! grid, numerical solver, edge policy, and implementation here are original.

use crate::colored_noise::Biquad;
use std::f64::consts::TAU;

const PEAKS: usize = 12;
const POINTS: usize = 120;
const ITERATIONS: usize = 12;

fn normalized(b: [f64; 3], a: [f64; 3]) -> Biquad {
    Biquad {
        b: b.map(|v| v / a[0]),
        a: [a[1] / a[0], a[2] / a[0]],
    }
}

fn peak(frequency: f64, rate: f64, gain_db: f64) -> Biquad {
    let (sine, cosine) = (TAU * frequency / rate).sin_cos();
    let alpha = sine * 0.5; // Q = 1
    let gain = 10.0_f64.powf(gain_db / 40.0);
    normalized(
        [1.0 + alpha * gain, -2.0 * cosine, 1.0 - alpha * gain],
        [1.0 + alpha / gain, -2.0 * cosine, 1.0 - alpha / gain],
    )
}

fn edge(frequency: f64, rate: f64, q: f64, high_pass: bool) -> Biquad {
    let (sine, cosine) = (TAU * frequency / rate).sin_cos();
    let alpha = sine / (2.0 * q);
    let b = if high_pass {
        [(1.0 + cosine) * 0.5, -1.0 - cosine, (1.0 + cosine) * 0.5]
    } else {
        [(1.0 - cosine) * 0.5, 1.0 - cosine, (1.0 - cosine) * 0.5]
    };
    normalized(b, [1.0 + alpha, -2.0 * cosine, 1.0 - alpha])
}

fn response_db(section: &Biquad, omega: f64) -> f64 {
    let (s1, c1) = omega.sin_cos();
    let (s2, c2) = (2.0 * omega).sin_cos();
    let real_b = section.b[0] + section.b[1] * c1 + section.b[2] * c2;
    let imag_b = section.b[1] * s1 + section.b[2] * s2;
    let real_a = 1.0 + section.a[0] * c1 + section.a[1] * c2;
    let imag_a = section.a[0] * s1 + section.a[1] * s2;
    10.0 * ((real_b * real_b + imag_b * imag_b) / (real_a * real_a + imag_a * imag_a))
        .max(1e-100)
        .log10()
}

fn inverse_a_db(frequency: f64) -> f64 {
    let f2 = frequency * frequency;
    let amplitude = (f2 + 20.6_f64.powi(2))
        * ((f2 + 107.7_f64.powi(2)) * (f2 + 737.9_f64.powi(2))).sqrt()
        * (f2 + 12194.0_f64.powi(2))
        / (12194.0_f64.powi(2) * f2 * f2);
    20.0 * amplitude.log10()
}

/// Solve a ridge-regularized normal system using partial pivoting. All arrays
/// have fixed size; the fitter is only called when the graph is compiled.
fn solve(mut augmented: [[f64; PEAKS + 1]; PEAKS]) -> [f64; PEAKS] {
    for column in 0..PEAKS {
        let pivot = (column..PEAKS)
            .max_by(|&a, &b| {
                augmented[a][column]
                    .abs()
                    .total_cmp(&augmented[b][column].abs())
            })
            .unwrap();
        augmented.swap(column, pivot);
        let divisor = augmented[column][column];
        if divisor.abs() < 1e-14 || !divisor.is_finite() {
            return [0.0; PEAKS];
        }
        for value in &mut augmented[column][column..=PEAKS] {
            *value /= divisor;
        }
        let pivot_row = augmented[column];
        for (row_index, row) in augmented.iter_mut().enumerate() {
            if row_index != column {
                let factor = row[column];
                for index in column..=PEAKS {
                    row[index] -= factor * pivot_row[index];
                }
            }
        }
    }
    std::array::from_fn(|index| augmented[index][PEAKS])
}

/// Fit 80 Hz .. min(20 kHz, 0.4 Fs). Outside this interval the response has
/// deliberately soft edges: fourth-order HP at 60 Hz and fourth-order LP at
/// min(22 kHz, 0.46 Fs). The caller supplies ensemble-RMS normalization.
pub(crate) fn design(rate: u32) -> [Biquad; 16] {
    let rate = f64::from(rate);
    let upper = 20000.0_f64.min(0.4 * rate);
    let centers: [f64; PEAKS] =
        std::array::from_fn(|i| 80.0 * (upper / 80.0).powf(i as f64 / (PEAKS - 1) as f64));
    let frequencies: [f64; POINTS] =
        std::array::from_fn(|i| 80.0 * (upper / 80.0).powf(i as f64 / (POINTS - 1) as f64));
    let omega = frequencies.map(|f| TAU * f / rate);
    let q_low = 1.0 / (2.0 * (std::f64::consts::PI / 8.0).cos());
    let q_high = 1.0 / (2.0 * (3.0 * std::f64::consts::PI / 8.0).cos());
    let edges = [
        edge(60.0, rate, q_low, true),
        edge(60.0, rate, q_high, true),
        edge(22000.0_f64.min(0.46 * rate), rate, q_low, false),
        edge(22000.0_f64.min(0.46 * rate), rate, q_high, false),
    ];
    let targets: [f64; POINTS] = std::array::from_fn(|i| {
        inverse_a_db(frequencies[i])
            - inverse_a_db(1000.0)
            - edges.iter().map(|b| response_db(b, omega[i])).sum::<f64>()
    });
    let mut gains = [0.0; PEAKS];
    for _ in 0..ITERATIONS {
        let responses: [[f64; POINTS]; PEAKS] = std::array::from_fn(|i| {
            let section = peak(centers[i], rate, gains[i]);
            omega.map(|w| response_db(&section, w))
        });
        let residual: [f64; POINTS] =
            std::array::from_fn(|k| targets[k] - responses.iter().map(|row| row[k]).sum::<f64>());
        let jacobian: [[f64; POINTS]; PEAKS] = std::array::from_fn(|i| {
            let perturbed = peak(centers[i], rate, gains[i] + 0.05);
            std::array::from_fn(|k| (response_db(&perturbed, omega[k]) - responses[i][k]) / 0.05)
        });
        let system = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                if j == PEAKS {
                    jacobian[i].iter().zip(residual).map(|(a, b)| a * b).sum()
                } else {
                    jacobian[i]
                        .iter()
                        .zip(jacobian[j])
                        .map(|(a, b)| a * b)
                        .sum::<f64>()
                        + if i == j { 0.001 } else { 0.0 }
                }
            })
        });
        let delta = solve(system);
        for i in 0..PEAKS {
            gains[i] = (gains[i] + 0.8 * delta[i]).clamp(-40.0, 40.0);
        }
    }
    std::array::from_fn(|i| {
        if i < 4 {
            edges[i]
        } else {
            peak(centers[i - 4], rate, gains[i - 4])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_a_shape_and_pole_stability_across_supported_rates() {
        for rate in [
            8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 192000,
        ] {
            let sections = design(rate);
            for section in &sections {
                assert!(section.b.iter().chain(&section.a).all(|v| v.is_finite()));
                // Jury stability conditions for z² + a1 z + a2.
                let [a1, a2] = section.a;
                assert!(a2.abs() < 1.0 && 1.0 + a1 + a2 > 0.0 && 1.0 - a1 + a2 > 0.0);
            }
            let rate = f64::from(rate);
            let reference = sections
                .iter()
                .map(|b| response_db(b, TAU * 1000.0 / rate))
                .sum::<f64>();
            let upper = 20000.0_f64.min(0.4 * rate);
            // A denser, offset logarithmic grid than the fitting grid tests
            // interpolation as well as fitted frequencies.
            for i in 0..1000 {
                let f = 80.0 * (upper / 80.0).powf((i as f64 + 0.37) / 1000.0);
                let measured = sections
                    .iter()
                    .map(|b| response_db(b, TAU * f / rate))
                    .sum::<f64>()
                    - reference;
                let expected = inverse_a_db(f) - inverse_a_db(1000.0);
                assert!(
                    (measured - expected).abs() < 1.7,
                    "{rate} Hz, {f} Hz: {} dB error",
                    measured - expected
                );
            }
        }
    }
}
