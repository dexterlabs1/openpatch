use super::*;

fn noise(rate: u32, seed: u32, level: f32) -> ColoredNoise {
    ColoredNoise::build(
        &Node {
            content: None,
            rackmcp_sequence: None,
            score: None,
            bypassed: false,
            id: "noise".into(),
            kind: "colored_noise".into(),
            params: [("seed".into(), seed as f32), ("level".into(), level)].into(),
        },
        rate,
    )
    .unwrap()
}

#[test]
fn transfer_slopes_and_normalization_cover_supported_sample_rates() {
    for rate in [
        8000, 11025, 16000, 22050, 32000, 44100, 48000, 96000, 192000,
    ] {
        let c = coefficients(rate);
        let maximum = (f64::from(rate) / 8.0).min(10000.0);
        for (color, expected) in [(0, 0.0), (1, -3.0), (2, -6.0), (3, 6.0), (4, 3.0), (6, 0.0)] {
            let mut f = 100.0;
            while f * 2.0 <= maximum {
                let low = colored_noise_response_db(OUTPUTS[color], f, rate).unwrap();
                let high = colored_noise_response_db(OUTPUTS[color], 2.0 * f, rate).unwrap();
                assert!(
                    (high - low - expected).abs() < 0.5,
                    "rate={rate},color={},frequency={f},slope={}",
                    OUTPUTS[color],
                    high - low
                );
                f *= 2.0;
            }
        }
        // Independent midpoint integration on a different logarithmic grid.
        let n = 12000;
        let low = 1e-9_f64;
        let span = (PI / low).ln();
        for color in 0..6 {
            let power: f64 = (0..n)
                .map(|i| {
                    let left = low * (span * i as f64 / n as f64).exp();
                    let right = low * (span * (i + 1) as f64 / n as f64).exp();
                    c.power(color, (left * right).sqrt()) * (right - left)
                })
                .sum();
            let rms = (power / PI).sqrt() * c.gain[color];
            assert!(
                (rms / NOMINAL_RMS - 1.0).abs() < 0.01,
                "{rate}, {color}, {rms}"
            );
        }
    }
}

#[test]
fn sampled_distribution_rms_dc_and_correlations_match_noise_roles() {
    const COUNT: usize = 1_440_000; // 30 s; slowest stochastic bands need duration.
    let mut source = noise(48000, 7391, 1.0);
    for _ in 0..48000 {
        source.values();
    }
    let mut sum = [0.0; 7];
    let mut square = [0.0; 7];
    let mut lag = [0.0; 7];
    let mut previous = [0.0; 7];
    let mut white_pink = 0.0;
    let mut white_black = 0.0;
    let mut fourth_white = 0.0;
    let mut black_bins = [0usize; 10];
    let mut gaussian_peak = 0.0_f64;
    for _ in 0..COUNT {
        let values = source.values().map(f64::from);
        for i in 0..7 {
            sum[i] += values[i];
            square[i] += values[i] * values[i];
            lag[i] += values[i] * previous[i];
        }
        white_pink += values[0] * values[1];
        white_black += values[0] * values[6];
        fourth_white += values[0].powi(4);
        gaussian_peak = gaussian_peak.max(values[0].abs());
        assert!(values.iter().all(|v| v.is_finite()));
        assert!(values[6].abs() <= 5.0);
        black_bins[((values[6] + 5.0).floor() as usize).min(9)] += 1;
        previous = values;
    }
    for color in 0..7 {
        let mean = sum[color] / COUNT as f64;
        let rms = (square[color] / COUNT as f64).sqrt();
        let expected = if color == 6 {
            5.0 / 3.0_f64.sqrt()
        } else {
            NOMINAL_RMS
        };
        let tolerance = if matches!(color, 1 | 2 | 5) {
            0.10
        } else {
            0.05
        };
        assert!(
            (rms / expected - 1.0).abs() < tolerance,
            "{} RMS {rms}",
            OUTPUTS[color]
        );
        // Slow colored spectra have a larger finite-observation mean variance.
        assert!(
            mean.abs()
                < expected
                    * if matches!(color, 1 | 2 | 5) {
                        0.15
                    } else {
                        0.01
                    },
            "{} mean {mean}",
            OUTPUTS[color]
        );
    }
    let correlation = |i: usize| lag[i] / square[i];
    assert!(correlation(0).abs() < 0.01);
    assert!(correlation(6).abs() < 0.01);
    assert!((correlation(3) + 0.5).abs() < 0.01);
    assert!(correlation(2) > 0.99);
    assert!(correlation(1) > 0.5);
    assert!(correlation(4) < 0.0);
    assert!((white_pink / (square[0] * square[1]).sqrt()).abs() < 0.01);
    assert!((white_black / (square[0] * square[6]).sqrt()).abs() < 0.01);
    let kurtosis = fourth_white / COUNT as f64 / (square[0] / COUNT as f64).powi(2);
    assert!((kurtosis - 3.0).abs() < 0.05);
    assert!(
        gaussian_peak > 10.0,
        "Gaussian output must not be peak-clipped"
    );
    for count in black_bins {
        assert!((count as f64 / (COUNT as f64 / 10.0) - 1.0).abs() < 0.015);
    }
}

#[test]
fn level_scales_every_color_without_affecting_stream_state() {
    let mut unity = noise(44100, 0, 1.0);
    let mut double = noise(44100, 0, 2.0);
    let mut silent = noise(44100, 0, 0.0);
    for _ in 0..20001 {
        let a = unity.values();
        let b = double.values();
        let c = silent.values();
        for i in 0..7 {
            assert_eq!(b[i], 2.0 * a[i]);
            assert_eq!(c[i], 0.0);
        }
    }
    silent.level = 1.0;
    assert_eq!(silent.values(), unity.values());
}

#[test]
fn exporter_writes_unclipped_production_samples() {
    // Deliberately cheap smoke test: checks the actual seven-output callback.
    let mut source = noise(48000, 1, 1.0);
    let mut output = [PortFrame::default(); 7];
    source.process(&mut output);
    assert!(output
        .iter()
        .all(|f| f.channels == 1 && f.values[1..].iter().all(|v| *v == 0.0)));
}

/// Root qualification exports raw little-endian f32 frames in OUTPUTS order.
#[test]
#[ignore = "writes env-selected raw sample artifact for external spectral qualification"]
fn export_colored_noise_samples() {
    use std::io::{BufWriter, Write};
    let rate: u32 = std::env::var("OPENPATCH_NOISE_RATE")
        .unwrap_or_else(|_| "48000".into())
        .parse()
        .unwrap();
    let seed: u32 = std::env::var("OPENPATCH_NOISE_SEED")
        .unwrap_or_else(|_| "7391".into())
        .parse()
        .unwrap();
    let seconds: f64 = std::env::var("OPENPATCH_NOISE_SECONDS")
        .unwrap_or_else(|_| "30".into())
        .parse()
        .unwrap();
    let warmup: f64 = std::env::var("OPENPATCH_NOISE_WARMUP")
        .unwrap_or_else(|_| "1".into())
        .parse()
        .unwrap();
    let path = std::env::var("OPENPATCH_NOISE_OUTPUT").expect("OPENPATCH_NOISE_OUTPUT required");
    assert!(
        (8000..=192000).contains(&rate)
            && seed <= 16777215
            && seconds > 0.0
            && seconds <= 300.0
            && (0.0..=60.0).contains(&warmup)
    );
    let mut source = noise(rate, seed, 1.0);
    for _ in 0..(warmup * f64::from(rate)) as usize {
        source.values();
    }
    let mut writer = BufWriter::new(std::fs::File::create(path).unwrap());
    for _ in 0..(seconds * f64::from(rate)) as usize {
        for value in source.values() {
            writer.write_all(&value.to_le_bytes()).unwrap();
        }
    }
    writer.flush().unwrap();
    println!("seven interleaved channels: {}", OUTPUTS.join(","));
}

#[test]
fn invalid_response_requests_are_rejected() {
    for (port, hz, rate) in [
        ("out", 1000.0, 48000),
        ("gray", 0.0, 48000),
        ("gray", f64::NAN, 48000),
        ("blue", 24000.0, 48000),
        ("red", 1000.0, 7999),
    ] {
        assert!(colored_noise_response_db(port, hz, rate).is_none());
    }
}
