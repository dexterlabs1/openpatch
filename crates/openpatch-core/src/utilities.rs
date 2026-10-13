//! Original mono utility algorithms. These implement the contracts in catalog(),
//! not complete VCV modules. The official manuals were consulted for functional
//! roles and voltage conventions; no plugin implementation code is incorporated.

use crate::{bounded, parameter, Node};
use serde_json::{json, Value};
use std::f64::consts::TAU;

pub(crate) fn catalog() -> [Value; 5] {
    [
        json!({"kind":"quantizer","title":"QUANTIZER","model":"functional","description":"Mono 1 V/oct nearest enabled note. scale_mask bits C..B; ties choose higher pitch. Empty scale bypasses adjusted input. offset + offset_cv are applied before quantizing.","inputs":["pitch","offset_cv"],"params":{
            "scale_mask":{"default":4095.0,"min":0.0,"max":4095.0,"step":1.0,"unit":"12-bit mask"},
            "offset":{"default":0.0,"min":-4.0,"max":4.0,"unit":"V"}}}),
        json!({"kind":"sample_hold","title":"S&H","model":"functional","description":"One sample-and-hold: captures signal on trigger rising through 1 V; rearms at or below 0.1 V. Held output starts at initial; no droop. Graph edits preserve voltage and trigger state; loading a patch starts at initial.","inputs":["signal","trigger"],"params":{
            "initial":{"default":0.0,"min":-16.0,"max":16.0,"unit":"V"}}}),
        json!({"kind":"white_noise","title":"WHITE NOISE","model":"functional","description":"Deterministic seeded Gaussian white noise. level sets RMS volts; no peak limiter. Seed edits restart the sequence; other graph edits preserve it. Single output, no colored-noise modes.","inputs":[],"params":{
            "seed":{"default":1.0,"min":0.0,"max":16777215.0,"step":1.0},
            "level":{"default":1.0,"min":0.0,"max":10.0,"unit":"V RMS"}}}),
        json!({"kind":"attenuverter","title":"ATTENUVERTER","model":"functional","description":"Single affine voltage utility: gain * signal + offset. Unpatched signal is 10 V. Arithmetic safety bound ±1,000,000 V; no slew or output range control.","inputs":["signal"],"normalizations":{"signal":10.0},"params":{
            "gain":{"default":0.0,"min":-1.0,"max":1.0,"unit":"x"},
            "offset":{"default":0.0,"min":-10.0,"max":10.0,"unit":"V"}}}),
        json!({"kind":"crossfade","title":"CROSSFADE","model":"functional","description":"Two-input, one-output crossfade. mix 0 selects a; mix 1 selects b. law 0 is linear, 1 uses square-root gains (constant power for uncorrelated inputs). No CV port or second output; ±1,000,000 V safety bound.","inputs":["a","b"],"params":{
            "mix":{"default":0.5,"min":0.0,"max":1.0,"unit":"ratio"},
            "law":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}}}),
    ]
}

pub(crate) fn allowed(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "quantizer" => Some(&["scale_mask", "offset"]),
        "sample_hold" => Some(&["initial"]),
        "white_noise" => Some(&["seed", "level"]),
        "attenuverter" => Some(&["gain", "offset"]),
        "crossfade" => Some(&["mix", "law"]),
        _ => None,
    }
}

fn integer_parameter(node: &Node, name: &str, default: f32, max: f32) -> Result<u32, String> {
    let value = parameter(node, name, default, 0.0, max)?;
    if value.fract() != 0.0 {
        return Err(format!(
            "node '{}': parameter '{name}' must be an integer",
            node.id
        ));
    }
    Ok(value as u32)
}

fn finite_voltage(value: f32) -> f32 {
    if value.is_finite() {
        bounded(value)
    } else {
        0.0
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Utility {
    Quantizer { mask: u16, offset: f32 },
    SampleHold { held: f32, high: bool },
    WhiteNoise(WhiteNoise),
    Attenuverter { gain: f32, offset: f32 },
    Crossfade { gain_a: f32, gain_b: f32 },
}

impl Utility {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        Ok(match node.kind.as_str() {
            "quantizer" => Self::Quantizer {
                mask: integer_parameter(node, "scale_mask", 4095.0, 4095.0)? as u16,
                offset: parameter(node, "offset", 0.0, -4.0, 4.0)?,
            },
            "sample_hold" => Self::SampleHold {
                held: parameter(node, "initial", 0.0, -16.0, 16.0)?,
                high: false,
            },
            "white_noise" => Self::WhiteNoise(WhiteNoise::new(
                integer_parameter(node, "seed", 1.0, 16777215.0)?,
                parameter(node, "level", 1.0, 0.0, 10.0)?,
            )),
            "attenuverter" => Self::Attenuverter {
                gain: parameter(node, "gain", 0.0, -1.0, 1.0)?,
                offset: parameter(node, "offset", 0.0, -10.0, 10.0)?,
            },
            "crossfade" => {
                let mix = parameter(node, "mix", 0.5, 0.0, 1.0)?;
                let law = integer_parameter(node, "law", 0.0, 1.0)?;
                let (gain_a, gain_b) = if law == 0 {
                    (1.0 - mix, mix)
                } else {
                    ((1.0 - mix).sqrt(), mix.sqrt())
                };
                Self::Crossfade { gain_a, gain_b }
            }
            _ => unreachable!("validated utility kind"),
        })
    }
    pub(crate) fn port(&self, name: &str) -> Option<usize> {
        match (self, name) {
            (Self::Quantizer { .. }, "pitch")
            | (Self::SampleHold { .. }, "signal")
            | (Self::Attenuverter { .. }, "signal")
            | (Self::Crossfade { .. }, "a") => Some(0),
            (Self::Quantizer { .. }, "offset_cv")
            | (Self::SampleHold { .. }, "trigger")
            | (Self::Crossfade { .. }, "b") => Some(1),
            _ => None,
        }
    }
    pub(crate) fn copy_state_from(&mut self, previous: &Self) {
        match (self, previous) {
            (
                Self::SampleHold { held, high },
                Self::SampleHold {
                    held: old_held,
                    high: old_high,
                },
            ) => {
                *held = *old_held;
                *high = *old_high;
            }
            (Self::WhiteNoise(current), Self::WhiteNoise(old)) if current.seed == old.seed => {
                current.state = old.state;
                current.spare = old.spare;
            }
            _ => {}
        }
    }
    #[inline]
    pub(crate) fn process(&mut self, a: f32, b: f32, input_a_connected: bool) -> f32 {
        match self {
            Self::Quantizer { mask, offset } => quantize(
                finite_voltage(a) as f64 + finite_voltage(b) as f64 + *offset as f64,
                *mask,
            ),
            Self::SampleHold { held, high } => {
                let trigger = finite_voltage(b);
                if !*high && trigger >= 1.0 {
                    *held = finite_voltage(a);
                    *high = true;
                } else if *high && trigger <= 0.1 {
                    *high = false;
                }
                *held
            }
            Self::WhiteNoise(noise) => noise.process(),
            Self::Attenuverter { gain, offset } => bounded(
                *gain
                    * if input_a_connected {
                        finite_voltage(a)
                    } else {
                        10.0
                    }
                    + *offset,
            ),
            Self::Crossfade { gain_a, gain_b } => {
                bounded(*gain_a * finite_voltage(a) + *gain_b * finite_voltage(b))
            }
        }
    }
}

fn quantize(volts: f64, mask: u16) -> f32 {
    let volts = volts.clamp(-1_000_000.0, 1_000_000.0);
    if mask == 0 {
        return volts as f32;
    }
    let semitones = volts * 12.0;
    let mut best = 0.0;
    let mut distance = f64::INFINITY;
    for note in 0..12 {
        if mask & (1 << note) == 0 {
            continue;
        }
        // Nearest occurrence of this pitch class across ALL octaves. floor(x+.5)
        // makes exact ties move upward for negative pitches as well as positive.
        let octave = ((semitones - note as f64) / 12.0 + 0.5).floor();
        let candidate = octave * 12.0 + note as f64;
        let error = (candidate - semitones).abs();
        if error < distance || (error == distance && candidate > best) {
            best = candidate;
            distance = error;
        }
    }
    bounded((best / 12.0) as f32)
}

#[derive(Clone, Debug)]
pub(crate) struct WhiteNoise {
    seed: u32,
    state: u64,
    spare: Option<f64>,
    level: f32,
}
impl WhiteNoise {
    fn new(seed: u32, level: f32) -> Self {
        Self {
            seed,
            state: seed as u64 + 1,
            spare: None,
            level,
        }
    }
    fn uniform_open(&mut self) -> f64 {
        // xorshift64* recurrence; seed+1 excludes the absorbing zero state.
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let bits = self.state.wrapping_mul(2_685_821_657_736_338_717);
        ((bits >> 12) as f64 + 0.5) / 4_503_599_627_370_496.0
    }
    fn process(&mut self) -> f32 {
        let normal = match self.spare.take() {
            Some(value) => value,
            None => {
                // Box-Muller transform: zero mean, unit variance before level.
                let radius = (-2.0 * self.uniform_open().ln()).sqrt();
                let (sine, cosine) = (TAU * self.uniform_open()).sin_cos();
                self.spare = Some(radius * sine);
                radius * cosine
            }
        };
        (normal * self.level as f64) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn utility(kind: &str, params: &[(&str, f32)]) -> Utility {
        Utility::build(&Node {
            id: "test".into(),
            kind: kind.into(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), *v))
                .collect::<BTreeMap<_, _>>(),
        })
        .unwrap()
    }
    #[test]
    fn every_scale_matches_independent_nearest_note_search_at_signed_octave_boundaries() {
        for mask in 1..=4095 {
            for voltage in [-2.25, -1.125, -0.125, 0.0, 0.125, 0.25, 0.875, 1.125, 2.25] {
                let pitch = voltage * 12.0;
                let mut closest = 0;
                let mut distance = f64::INFINITY;
                // Independent brute-force search across a sufficiently broad
                // semitone range, ordered low-to-high with ties replaced.
                for note in -48_i32..=48 {
                    if mask & (1 << note.rem_euclid(12)) == 0 {
                        continue;
                    }
                    let error = (pitch - note as f64).abs();
                    if error <= distance {
                        closest = note;
                        distance = error;
                    }
                }
                assert_eq!(
                    quantize(voltage, mask),
                    (closest as f64 / 12.0) as f32,
                    "mask={mask}, pitch={pitch}"
                );
            }
        }
    }
    #[test]
    fn quantizer_ties_empty_scale_offsets_and_membership_are_explicit() {
        assert_eq!(quantize(0.125, 4095), (2.0 / 12.0) as f32);
        assert_eq!(quantize(-0.125, 4095), (-1.0 / 12.0) as f32);
        assert_eq!(quantize(0.25, 1 | (1 << 6)), 0.5);
        assert_eq!(quantize(-0.25, 1 | (1 << 6)), 0.0);
        let mut empty = utility("quantizer", &[("scale_mask", 0.0), ("offset", 0.25)]);
        assert_eq!(empty.process(-0.75, 0.25, true), -0.25);
        let mut shifted = utility("quantizer", &[("scale_mask", 1.0), ("offset", 0.25)]);
        assert_eq!(shifted.process(0.2, 0.1, true), 1.0);
    }
    #[test]
    fn sample_hold_samples_once_per_hysteretic_rising_edge() {
        let mut sampler = utility("sample_hold", &[("initial", -3.0)]);
        let triggers = [0.0, 0.99, 1.0, 0.9, 1.1, 0.11, 0.1, 0.99, 1.0];
        let expected = [-3.0, -3.0, 4.0, 4.0, 4.0, 4.0, 4.0, 4.0, 10.0];
        for (n, (&trigger, &held)) in triggers.iter().zip(&expected).enumerate() {
            assert_eq!(sampler.process(n as f32 + 2.0, trigger, true), held);
        }
        for _ in 0..1000 {
            assert_eq!(sampler.process(-9.0, 10.0, true), 10.0);
        }
        assert_eq!(sampler.process(-9.0, 0.0, true), 10.0);
        assert_eq!(sampler.process(-9.0, 10.0, true), -9.0);
    }
    #[test]
    fn attenuverter_normalization_affine_mapping_and_crossfade_gain_laws() {
        let mut inverter = utility("attenuverter", &[("gain", -0.5), ("offset", 2.0)]);
        assert_eq!(inverter.process(6.0, 0.0, true), -1.0);
        assert_eq!(inverter.process(0.0, 0.0, false), -3.0);
        for law in [0.0, 1.0] {
            let mut a = utility("crossfade", &[("mix", 0.0), ("law", law)]);
            let mut b = utility("crossfade", &[("mix", 1.0), ("law", law)]);
            assert_eq!(a.process(-3.0, 7.0, true), -3.0);
            assert_eq!(b.process(-3.0, 7.0, true), 7.0);
            let mut center = utility("crossfade", &[("law", law)]);
            let expected = if law == 0.0 {
                2.0
            } else {
                2.0 * 2.0_f32.sqrt()
            };
            assert!((center.process(-3.0, 7.0, true) - expected).abs() < 1e-6);
        }
    }
    #[test]
    fn seeded_gaussian_noise_distribution_rms_and_lag_correlations() {
        let mut noise = WhiteNoise::new(2917, 2.0);
        let count = 131072;
        let values: Vec<f64> = (0..count).map(|_| noise.process() as f64).collect();
        let mean = values.iter().sum::<f64>() / count as f64;
        let power = values.iter().map(|x| x * x).sum::<f64>() / count as f64;
        let fourth = values.iter().map(|x| x.powi(4)).sum::<f64>() / count as f64;
        assert!(mean.abs() < 0.025, "mean={mean}");
        assert!(
            (power.sqrt() / 2.0 - 1.0).abs() < 0.01,
            "RMS={}",
            power.sqrt()
        );
        assert!(
            (fourth / (power * power) - 3.0).abs() < 0.10,
            "kurtosis={}",
            fourth / (power * power)
        );
        for lag in 1..=16 {
            let correlation = values[lag..]
                .iter()
                .zip(&values[..count - lag])
                .map(|(x, y)| x * y)
                .sum::<f64>()
                / ((count - lag) as f64 * power);
            assert!(
                correlation.abs() < 0.012,
                "lag={lag} correlation={correlation}"
            );
        }
        let mut a = WhiteNoise::new(0, 1.0);
        let mut b = WhiteNoise::new(0, 1.0);
        let mut other = WhiteNoise::new(1, 1.0);
        let mut different = 0;
        for _ in 0..1024 {
            let sample = a.process();
            assert_eq!(sample, b.process());
            different += usize::from(sample != other.process());
        }
        assert_eq!(different, 1024);
    }
    #[test]
    fn state_copy_preserves_spare_sample_and_seed_edits_restart_noise() {
        let mut old = utility("white_noise", &[("seed", 17.0)]);
        for _ in 0..17 {
            old.process(0.0, 0.0, false);
        }
        let mut continued = utility("white_noise", &[("seed", 17.0), ("level", 2.0)]);
        continued.copy_state_from(&old);
        for _ in 0..20 {
            assert_eq!(
                continued.process(0.0, 0.0, false),
                2.0 * old.process(0.0, 0.0, false)
            );
        }
        let mut changed = utility("white_noise", &[("seed", 18.0)]);
        let mut fresh = utility("white_noise", &[("seed", 18.0)]);
        changed.copy_state_from(&old);
        for _ in 0..20 {
            assert_eq!(
                changed.process(0.0, 0.0, false),
                fresh.process(0.0, 0.0, false)
            );
        }
        let mut sampler = utility("sample_hold", &[]);
        sampler.process(3.0, 10.0, true);
        let mut copied = utility("sample_hold", &[("initial", -2.0)]);
        copied.copy_state_from(&sampler);
        assert_eq!(copied.process(7.0, 10.0, true), 3.0);
        copied.process(7.0, 0.0, true);
        assert_eq!(copied.process(7.0, 10.0, true), 7.0);
    }
    #[test]
    fn nonfinite_utility_inputs_have_explicit_zero_policy() {
        let mut inverter = utility("attenuverter", &[("gain", 1.0)]);
        assert_eq!(inverter.process(f32::INFINITY, 0.0, true), 0.0);
        let mut quantizer = utility("quantizer", &[]);
        assert_eq!(quantizer.process(f32::NAN, f32::INFINITY, true), 0.0);
        let mut sampler = utility("sample_hold", &[]);
        assert_eq!(sampler.process(f32::NAN, 10.0, true), 0.0);
    }
}
