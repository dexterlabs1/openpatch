// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from VCV Fundamental, Andrew Belt and contributors; see LICENSE.
//! Fundamental 2.6.4's non-oversampled, rational-clipped RK4 ladder VCF.
//!
//! The source leaves LadderFilter::input uninitialized at construction. This
//! implementation starts every lane's previous input at zero, including groups
//! activated later. Undefined upstream cold-start behavior is not an equivalence
//! claim. Reference harnesses must explicitly record their initial input state.
use crate::{parameter, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 4] = ["cutoff_cv", "resonance_cv", "drive_cv", "signal"];
pub(crate) const OUTPUTS: [&str; 2] = ["lowpass", "highpass"];
const NAMES: [&str; 6] = [
    "frequency",
    "resonance",
    "cutoff_cv_amount",
    "resonance_cv_amount",
    "drive",
    "drive_cv_amount",
];
const C4: f32 = 261.6256;
const SEED: [u64; 2] = [0x123456789abcdef0, 0xfedcba9876543210];

fn frequency_bounds() -> (f32, f32) {
    (
        ((C4 / 8000.0).log2() + 5.0) / 10.0,
        ((8000.0 / C4).log2() + 5.0) / 10.0,
    )
}

pub(crate) fn catalog() -> Value {
    let (min, max) = frequency_bounds();
    json!({"kind":"rack264_vcf","title":"Rack 2.6.4 VCF","model":"functional","inputs":INPUTS,"params":{
        "frequency":{"default":0.5,"min":min,"max":max,"display":{"scale":"exponential","base":1024,"multiplier":C4/32.0,"unit":"Hz"}},
        "resonance":{"default":0.0,"min":0.0,"max":1.0},
        "cutoff_cv_amount":{"default":0.0,"min":-1.0,"max":1.0},
        "resonance_cv_amount":{"default":0.0,"min":-1.0,"max":1.0},
        "drive":{"default":0.0,"min":-1.0,"max":1.0},
        "drive_cv_amount":{"default":0.0,"min":-1.0,"max":1.0}},
        "description":"Fundamental 2.6.4 digital four-pole ladder VCF with RK4 integration, rational clipping, simultaneous lowpass and highpass, signed CV depths and self-oscillation; no oversampling."})
}

pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["primary_input"] = json!("signal");
    module["polyphony"]["version_2"] = json!("signal width, minimum one; rounded groups of four; raw audio does not broadcast; mono CV broadcasts");
    module["polyphony"]["retired_voices"] =
        json!("inactive whole groups freeze; padding lanes in final group continue");
    if let Some(metadata) = module["input_metadata"].as_array_mut() {
        for meta in metadata {
            meta["channel_policy"] = json!(if meta["name"] == "signal" {
                "raw channels; mono does not broadcast"
            } else {
                "mono broadcasts; missing short-poly channels are zero"
            });
        }
    }
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/VCF.cpp");
    module["connectivity_policy"] = json!("both outputs disconnected freezes all filter history and RNG; either connected output advances the shared filter; disconnected output frames are zero");
    module["noise_policy"] = json!("-120 dB scalar noise once per processed four-lane group; deterministic module-local SDK-algorithm xoroshiro128+ seed, intentionally independent of Rack's process-global RNG; reset preserves the established stream");
    module["state_adoption"] = json!("same ID/kind/sample rate preserves all four filter states, previous input and RNG through parameter and cable edits; rate changes start fresh");
    module["initialization_policy"] = json!("construction deterministically zeros all four states and previous input in every group; upstream C++ previous input is uninitialized, so undefined cold-start equivalence is unclaimed; initialize/reset clear four states but preserve established previous input and RNG");
    module["cutoff_policy"] = json!("source knob bounds use log2(C4/8000) and log2(8000/C4); processing cutoff is limited to 1 Hz through 0.18 times sample rate");
    module["bypass_policy"] = json!("raw signal routes to both outputs; all DSP and RNG freeze");
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Lane {
    state: [f32; 4],
    input: f32,
}

fn clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

impl Lane {
    fn derivative(
        &self,
        input: f32,
        t: f32,
        dt: f32,
        x: [f32; 4],
        omega: f32,
        resonance: f32,
    ) -> [f32; 4] {
        let input_t = self.input + (input - self.input) * (t / dt);
        let input_c = clip(input_t - resonance * x[3]);
        let y = x.map(clip);
        [
            omega * (input_c - y[0]),
            omega * (y[0] - y[1]),
            omega * (y[1] - y[2]),
            omega * (y[2] - y[3]),
        ]
    }

    fn process(&mut self, input: f32, cutoff: f32, resonance: f32, dt: f32, mask: u16) -> [f32; 2] {
        let omega = 2.0 * std::f32::consts::PI * cutoff;
        // Preserve SDK stepRK4's arithmetic ordering, including multiplication
        // before division and the interpolation times passed to the derivative.
        let k1 = self.derivative(input, 0.0, dt, self.state, omega, resonance);
        let mut intermediate = [0.0; 4];
        for i in 0..4 {
            intermediate[i] = self.state[i] + k1[i] * dt / 2.0;
        }
        let k2 = self.derivative(input, dt / 2.0, dt, intermediate, omega, resonance);
        for i in 0..4 {
            intermediate[i] = self.state[i] + k2[i] * dt / 2.0;
        }
        let k3 = self.derivative(input, dt / 2.0, dt, intermediate, omega, resonance);
        for i in 0..4 {
            intermediate[i] = self.state[i] + k3[i] * dt;
        }
        let k4 = self.derivative(input, dt, dt, intermediate, omega, resonance);
        for i in 0..4 {
            self.state[i] += dt * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]) / 6.0;
        }
        self.input = input;
        [
            if mask & 1 != 0 {
                5.0 * self.state[3]
            } else {
                0.0
            },
            if mask & 2 != 0 {
                5.0 * clip(
                    (self.input - resonance * self.state[3]) - 4.0 * self.state[0]
                        + 6.0 * self.state[1]
                        - 4.0 * self.state[2]
                        + self.state[3],
                )
            } else {
                0.0
            },
        ]
    }
}

// SDK SIMD exp2Floor forms an IEEE exponent with integer bits, truncating
// after adding 127. Preserve source f32 coefficient rounding and Horner order.
#[allow(clippy::excessive_precision)]
fn exp2_taylor5(x: f32) -> f32 {
    let shifted = x + 127.0;
    let integer = shifted as i32;
    let fraction = shifted - integer as f32;
    let scale = f32::from_bits((integer as u32).wrapping_shl(23));
    let polynomial = 1.0
        + fraction
            * (0.69315169353961
                + fraction
                    * (0.2401595990753
                        + fraction
                            * (0.055817908652
                                + fraction * (0.008991698010 + fraction * 0.001879100722))));
    scale * polynomial
}

fn voltage(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}

#[derive(Debug)]
pub(crate) struct Rack264Vcf {
    parameters: [f32; 6],
    sample_time: f32,
    max_cutoff: f32,
    lanes: [Lane; 16],
    rng: [u64; 2],
}

impl Rack264Vcf {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if !NAMES.contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let (min, max) = frequency_bounds();
        let mut parameters = [0.0; 6];
        for i in 0..6 {
            let (default, min, max) = match i {
                0 => (0.5, min, max),
                1 => (0.0, 0.0, 1.0),
                _ => (0.0, -1.0, 1.0),
            };
            parameters[i] = parameter(node, NAMES[i], default, min, max)?;
        }
        if rate == 0 {
            return Err("rack264_vcf sample rate must be positive".into());
        }
        let mut module = Self {
            parameters,
            sample_time: 1.0 / rate as f32,
            max_cutoff: rate as f32 * 0.18,
            lanes: [Lane::default(); 16],
            rng: SEED,
        };
        // SDK seed() discards the first draw.
        module.draw();
        Ok(module)
    }

    #[allow(clippy::excessive_precision)]
    fn draw(&mut self) -> f32 {
        let [a, mut b] = self.rng;
        let result = a.wrapping_add(b);
        b ^= a;
        self.rng = [a.rotate_left(55) ^ b ^ (b << 14), b.rotate_left(36)];
        (result >> 32) as u32 as f32 * 2.32830629e-10
    }

    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_time == old.sample_time {
            self.lanes = old.lanes;
            self.rng = old.rng;
        }
    }

    pub(crate) fn initialize_state(&mut self) {
        for lane in &mut self.lanes {
            lane.state = [0.0; 4];
        }
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 4],
        outputs: &mut [PortFrame],
        connected_mask: u16,
    ) {
        let mask = connected_mask & 3;
        let channels = usize::from(inputs[3].channels).clamp(1, 16);
        for output in outputs.iter_mut().take(2) {
            *output = PortFrame::default();
            output.channels = channels as u8;
        }
        if mask == 0 {
            return;
        }
        let p = self.parameters;
        for group in 0..channels.div_ceil(4) {
            let noise = 1e-6 * (2.0 * self.draw() - 1.0);
            for lane in group * 4..group * 4 + 4 {
                let drive =
                    (p[4] + voltage(inputs[2].voice(lane, 0.0)) / 10.0 * p[5]).clamp(-1.0, 1.0);
                let base = 1.0 + drive;
                let squared = base * base;
                let gain = base * (squared * squared);
                let input = voltage(inputs[3].channel(lane)) / 5.0 * gain + noise;
                let resonance =
                    (p[1] + voltage(inputs[1].voice(lane, 0.0)) / 10.0 * p[3]).clamp(0.0, 1.0);
                let pitch = p[0] * 10.0 - 5.0 + voltage(inputs[0].voice(lane, 0.0)) * p[2];
                // f32::max/min give the source SIMD clamp's lower-bound result
                // for NaN from extreme exponent-bit approximation inputs.
                let cutoff = (C4 * exp2_taylor5(pitch)).max(1.0).min(self.max_cutoff);
                let result = self.lanes[lane].process(
                    input,
                    cutoff,
                    resonance * resonance * 10.0,
                    self.sample_time,
                    mask,
                );
                if lane < channels {
                    for output in 0..outputs.len().min(2) {
                        outputs[output].values[lane] = result[output];
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(params: &[(&str, f32)]) -> Node {
        Node {
            content: None,
            score: None,
            id: "v".into(),
            kind: "rack264_vcf".into(),
            bypassed: false,
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    fn model() -> Rack264Vcf {
        Rack264Vcf::build(&node(&[]), 48_000).unwrap()
    }

    fn input(width: u8) -> [PortFrame; 4] {
        let mut input = [PortFrame::default(); 4];
        input[3].channels = width;
        for lane in 0..usize::from(width) {
            input[3].values[lane] = 2.0;
        }
        input
    }

    fn step(m: &mut Rack264Vcf, input: [PortFrame; 4], mask: u16) -> [PortFrame; 2] {
        let mut out = [PortFrame::default(); 2];
        m.process(&input, &mut out, mask);
        for p in &out {
            assert!(p.values.iter().all(|v| v.is_finite()));
            assert!(p.values[usize::from(p.channels)..]
                .iter()
                .all(|v| *v == 0.0));
        }
        out
    }

    #[test]
    fn constructor_defaults_and_source_frequency_bounds() {
        let m = model();
        assert_eq!(m.parameters, [0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let (lo, hi) = frequency_bounds();
        assert!((C4 * (10.0 * lo - 5.0).exp2() - C4 * C4 / 8000.0).abs() < 1e-4);
        assert!((C4 * (10.0 * hi - 5.0).exp2() - 8000.0).abs() < 0.01);
        assert!(Rack264Vcf::build(&node(&[("frequency", lo)]), 48_000).is_ok());
        assert!(Rack264Vcf::build(&node(&[("frequency", hi)]), 48_000).is_ok());
        for (name, value) in [
            ("seed", 1.0),
            ("frequency", f32::NAN),
            ("resonance", f32::INFINITY),
            ("drive", 1.01),
            ("cutoff_cv_amount", -1.01),
            ("frequency", lo - 0.001),
            ("frequency", hi + 0.001),
        ] {
            assert!(Rack264Vcf::build(&node(&[(name, value)]), 48_000).is_err());
        }
        assert!(Rack264Vcf::build(&node(&[]), 0).is_err());
    }

    #[test]
    fn rational_clip_is_odd_and_saturates_at_three() {
        assert_eq!(clip(0.0), 0.0);
        assert_eq!(clip(3.0), 1.0);
        assert_eq!(clip(-3.0), -1.0);
        assert_eq!(clip(100.0), 1.0);
        assert_eq!(clip(-100.0), -1.0);
        assert_eq!(clip(1.0), 28.0 / 36.0);
    }

    #[test]
    fn established_previous_input_changes_first_step_and_reset_preserves_it() {
        let mut a = model();
        let mut b = model();
        a.lanes[0].input = -0.75;
        let oa = step(&mut a, input(1), 3);
        let ob = step(&mut b, input(1), 3);
        assert_ne!(oa[0].values[0], ob[0].values[0]);
        for _ in 0..19 {
            step(&mut a, input(5), 3);
        }
        let history = a.lanes.map(|lane| lane.input);
        let rng = a.rng;
        a.initialize_state();
        assert!(a.lanes.iter().all(|lane| lane.state == [0.0; 4]));
        assert_eq!(a.lanes.map(|lane| lane.input), history);
        assert_eq!(a.rng, rng);
        let mut fresh = model();
        fresh.initialize_state();
        assert_eq!(fresh.lanes, [Lane::default(); 16]);
    }

    #[test]
    fn sdk_rng_seed_warmup_and_one_draw_per_active_quartet() {
        let mut m = model();
        assert_eq!(
            m.draw().to_bits(),
            ((0x7e689c00_u32 as f32) * f32::from_bits(0x2f7fffff)).to_bits()
        );
        for (width, draws) in [(0, 1), (1, 1), (4, 1), (5, 2), (16, 4)] {
            let mut a = model();
            let mut b = model();
            step(&mut a, input(width), 3);
            for _ in 0..draws {
                b.draw();
            }
            assert_eq!(a.rng, b.rng);
        }
    }

    #[test]
    fn raw_audio_does_not_broadcast_but_mono_cv_does() {
        let mut a = model();
        let mut b = model();
        a.parameters[2] = 1.0;
        b.parameters[2] = 1.0;
        let mut i = input(1);
        i[0] = PortFrame::mono(1.0);
        let mut poly = i;
        poly[0].channels = 4;
        poly[0].values[..4].fill(1.0);
        step(&mut a, i, 3);
        step(&mut b, poly, 3);
        assert_eq!(a.lanes, b.lanes);
        assert_ne!(a.lanes[0], a.lanes[1]);
        assert_eq!(a.lanes[1], a.lanes[2]);
        assert_eq!(a.lanes[2], a.lanes[3]);
        assert_eq!(a.lanes[4], Lane::default());
    }

    #[test]
    fn padding_continues_and_whole_groups_freeze_then_regrow() {
        let mut m = model();
        for _ in 0..31 {
            step(&mut m, input(16), 3);
        }
        let old = m.lanes;
        step(&mut m, input(1), 3);
        assert_ne!(m.lanes[3], old[3]);
        assert_eq!(m.lanes[4..], old[4..]);
        step(&mut m, input(5), 3);
        assert_ne!(m.lanes[7], old[7]);
        assert_eq!(m.lanes[8..], old[8..]);
        let mut cold = model();
        step(&mut cold, input(1), 3);
        assert_eq!(cold.lanes[4..], [Lane::default(); 12]);
        let rng = cold.rng;
        let mut just_activated = model();
        just_activated.rng = rng;
        step(&mut cold, input(5), 3);
        step(&mut just_activated, input(5), 3);
        assert_eq!(cold.lanes[4..8], just_activated.lanes[4..8]);
    }

    #[test]
    fn disconnected_freeze_and_independent_output_masks() {
        let mut a = model();
        let mut b = model();
        for _ in 0..19 {
            step(&mut a, input(5), 1);
            step(&mut b, input(5), 2);
        }
        assert_eq!(a.lanes, b.lanes);
        assert_eq!(a.rng, b.rng);
        for _ in 0..27 {
            let out = step(&mut a, input(16), 0);
            assert_eq!(out[0].values, [0.0; 16]);
            assert_eq!(out[1].values, [0.0; 16]);
        }
        assert_eq!(a.lanes, b.lanes);
        assert_eq!(a.rng, b.rng);
        assert_eq!(step(&mut a, input(5), 3), step(&mut b, input(5), 3));
    }

    #[test]
    fn adoption_preserves_history_and_rng_but_keeps_new_parameters() {
        let mut old = model();
        for _ in 0..31 {
            step(&mut old, input(16), 3);
        }
        let mut next = Rack264Vcf::build(&node(&[("drive", 1.0)]), 48_000).unwrap();
        next.adopt(&old);
        assert_eq!(next.lanes, old.lanes);
        assert_eq!(next.rng, old.rng);
        assert_eq!(next.parameters[4], 1.0);
        let mut rate = Rack264Vcf::build(&node(&[]), 96_000).unwrap();
        let rng = rate.rng;
        rate.adopt(&old);
        assert_eq!(rate.lanes, [Lane::default(); 16]);
        assert_eq!(rate.rng, rng);
    }

    #[test]
    fn nonfinite_inputs_zero_and_extreme_cv_stays_finite() {
        let mut a = model();
        let mut b = model();
        let mut i = input(5);
        i[0] = PortFrame::mono(f32::NAN);
        i[1] = PortFrame::mono(f32::INFINITY);
        i[2] = PortFrame::mono(f32::NEG_INFINITY);
        i[3].values[0] = f32::NAN;
        let mut clean = i;
        clean[0] = PortFrame::mono(0.0);
        clean[1] = PortFrame::mono(0.0);
        clean[2] = PortFrame::mono(0.0);
        clean[3].values[0] = 0.0;
        assert_eq!(step(&mut a, i, 3), step(&mut b, clean, 3));
        a.parameters = [0.5, 1.0, 1.0, -1.0, 1.0, -1.0];
        for n in 0..256 {
            let mut i = input(16);
            for (p, port) in i.iter_mut().enumerate() {
                port.channels = 16;
                for (lane, v) in port.values.iter_mut().enumerate() {
                    *v = if (n + lane + p) % 2 == 0 {
                        f32::MAX
                    } else {
                        -f32::MAX
                    };
                }
            }
            step(&mut a, i, 3);
        }
    }
}
