//! Digital two-times oversampled ladder VCF. Equations and lifecycle are
//! qualified against the pinned Fundamental class; this is independent of the
//! circuit ladder model. See docs/STOCK_VCF.md and scripts/stock_vcf_reference.
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
// Same two-word xoroshiro128+ algorithm as Rack, with one initial discarded
// draw like SDK seed(). Per-module ownership is an intentional host deviation.
const SEED: [u64; 2] = [0x123456789abcdef0, 0xfedcba9876543210];
// Preserve the reference decimal coefficients and their f32 rounding.
#[allow(clippy::excessive_precision)]
const A: [f32; 3] = [0.062822416060049985, 0.4243808557204406, 0.7818614603969013];
// Preserve the reference decimal coefficients and their f32 rounding.
#[allow(clippy::excessive_precision)]
const B: [f32; 3] = [0.22380733034648345, 0.6165344350495111, 0.9274735948748258];
fn frequency_bounds() -> (f32, f32) {
    (
        ((8.0 / C4).log2() + 5.0) / 10.0,
        ((22000.0 / C4).log2() + 5.0) / 10.0,
    )
}
pub(crate) fn catalog() -> Value {
    let (min, max) = frequency_bounds();
    json!({"kind":"stock_vcf","title":"Stock VCF","model":"functional","inputs":INPUTS,"params":{
        "frequency":{"default":0.5,"min":min,"max":max,"display":{"scale":"exponential","base":1024,"multiplier":C4/32.0,"unit":"Hz"}},
        "resonance":{"default":0.0,"min":0.0,"max":1.0},
        "cutoff_cv_amount":{"default":0.0,"min":-1.0,"max":1.0},
        "resonance_cv_amount":{"default":0.0,"min":-1.0,"max":1.0},
        "drive":{"default":0.0,"min":-1.0,"max":1.0},
        "drive_cv_amount":{"default":0.0,"min":-1.0,"max":1.0}},
        "description":"Digital resonant four-pole ladder VCF with 2x IIR oversampling, simultaneous lowpass and highpass, signed CV depths and self-oscillation."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["primary_input"] = json!("signal");
    module["polyphony"]["version_2"]=json!("signal width, minimum one; rounded groups of four; raw audio does not broadcast; mono CV broadcasts");
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
    module["functional_reference"]=json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCF.cpp");
    module["connectivity_policy"]=json!("both outputs disconnected freezes all DSP and RNG; each disconnected output freezes its own decimator");
    module["noise_policy"]=json!("-120 dB scalar noise once per processed four-lane group; deterministic module-local SDK-algorithm xoroshiro128+ seed, intentionally independent of Rack's process-global RNG");
    module["state_adoption"]=json!("same ID/kind/sample rate preserves all filter/resampler histories and RNG through parameter and cable edits; rate changes start fresh");
    module["initialization_policy"] =
        json!("initialize/reset clear every filter and resampler and restore the fixed RNG seed");
    module["bypass_policy"] = json!("raw signal routes to both outputs; all DSP and RNG freeze");
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Allpass {
    a: [f32; 3],
    b: [f32; 3],
}
fn cascade(mut x: f32, state: &mut [f32; 3], coefficients: &[f32; 3]) -> f32 {
    for i in 0..3 {
        let y = coefficients[i] * x + state[i];
        state[i] = x - coefficients[i] * y;
        x = y;
    }
    x
}
impl Allpass {
    fn up(&mut self, x: f32) -> [f32; 2] {
        [cascade(x, &mut self.a, &A), cascade(x, &mut self.b, &B)]
    }
    fn down(&mut self, x: [f32; 2]) -> f32 {
        0.5 * (cascade(x[1], &mut self.a, &A) + cascade(x[0], &mut self.b, &B))
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Lane {
    up: Allpass,
    low: Allpass,
    high: Allpass,
    state: [f32; 4],
    previous: [f32; 4],
}
// Preserve the reference decimal coefficients and their f32 rounding.
#[allow(clippy::excessive_precision)]
fn tan_3_4(x: f32) -> f32 {
    let x2 = x * x;
    x * (1.0 + x2 * (-0.09776575533683811))
        / (1.0 + x2 * (-0.43119539396382 + x2 * 0.0105011966117302))
}
// Preserve the reference decimal coefficients and their f32 rounding.
#[allow(clippy::excessive_precision)]
fn tanh_ratio(x: f32) -> f32 {
    let x2 = x * x;
    (1.0 + x2 * (0.121953514066257 + x2 * 0.00204623480007919))
        / (1.0 + x2 * (0.455305674254515 + x2 * (0.0204552909446164 + x2 * 9.48027717633287e-5)))
}
// SDK SIMD exp2Floor deliberately forms the IEEE exponent through integer bits,
// and truncates after adding 127. Preserve its f32 rounding before Horner.
// Preserve the reference decimal coefficients and their f32 rounding.
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
impl Lane {
    fn process(&mut self, input: f32, cutoff: f32, resonance: f32, mask: u16) -> [f32; 2] {
        let g = tan_3_4(std::f32::consts::PI * (cutoff * 0.5));
        let oversampled = self.up.up(input);
        let mut low = [0.0; 2];
        let mut high = [0.0; 2];
        for n in 0..2 {
            let u = (oversampled[n] - resonance * self.previous[3]).clamp(-4.0, 4.0);
            let saturated = tanh_ratio(u) * u;
            let slopes = self.previous.map(|v| tanh_ratio(v).max(0.01));
            let y0 = (g * saturated + self.state[0]) / (1.0 + g * slopes[0]);
            let y1 = (g * slopes[0] * y0 + self.state[1]) / (1.0 + g * slopes[1]);
            let y2 = (g * slopes[1] * y1 + self.state[2]) / (1.0 + g * slopes[2]);
            let y3 = (g * slopes[2] * y2 + self.state[3]) / (1.0 + g * slopes[3]);
            self.previous = [y0, y1, y2, y3];
            for i in 0..4 {
                self.state[i] = 2.0 * self.previous[i] - self.state[i];
            }
            if mask & 1 != 0 {
                low[n] = y3 * tanh_ratio(y3 * 0.5);
            }
            if mask & 2 != 0 {
                let h = u - 4.0 * y0 + 6.0 * y1 - 4.0 * y2 + y3;
                high[n] = h * tanh_ratio(h * 0.5);
            }
        }
        [
            if mask & 1 != 0 {
                self.low.down(low) * 5.0
            } else {
                0.0
            },
            if mask & 2 != 0 {
                self.high.down(high) * 5.0
            } else {
                0.0
            },
        ]
    }
}
#[derive(Debug)]
pub(crate) struct StockVcf {
    parameters: [f32; 6],
    sample_time: f32,
    lanes: [Lane; 16],
    rng: [u64; 2],
}
impl StockVcf {
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
            return Err("stock_vcf sample rate must be positive".into());
        }
        let mut module = Self {
            parameters,
            sample_time: 1.0 / rate as f32,
            lanes: [Lane::default(); 16],
            rng: SEED,
        };
        module.draw();
        Ok(module)
    }
    // SDK random::get<float>() specifies this exact f32 conversion literal.
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
        self.lanes = [Lane::default(); 16];
        self.rng = SEED;
        self.draw();
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
                let r = (p[1] + voltage(inputs[1].voice(lane, 0.0)) / 10.0 * p[3]).clamp(0.0, 1.0);
                let pitch = p[0] * 10.0 - 5.0 + voltage(inputs[0].voice(lane, 0.0)) * p[2];
                // The source's exponent-bit approximation can yield NaN at
                // extreme CV. Host voltage policy guarantees finite output.
                let raw_cutoff = C4 * exp2_taylor5(pitch) * self.sample_time;
                let cutoff = if raw_cutoff.is_nan() {
                    0.0
                } else {
                    raw_cutoff.clamp(0.0, 0.499)
                };
                let result = self.lanes[lane].process(input, cutoff, r * r * 10.0, mask);
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
            rackmcp_sequence: None,
            score: None,
            id: "v".into(),
            kind: "stock_vcf".into(),
            bypassed: false,
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }
    fn model() -> StockVcf {
        StockVcf::build(&node(&[]), 48_000).unwrap()
    }
    fn input(width: u8) -> [PortFrame; 4] {
        let mut input = [PortFrame::default(); 4];
        input[3].channels = width;
        for lane in 0..usize::from(width) {
            input[3].values[lane] = 2.0;
        }
        input
    }
    fn step(m: &mut StockVcf, input: [PortFrame; 4], mask: u16) -> [PortFrame; 2] {
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
    fn parameters_reject_unknown_nonfinite_and_outside_bounds() {
        for (name, value) in [
            ("seed", 1.0),
            ("frequency", f32::NAN),
            ("resonance", f32::INFINITY),
            ("drive", 1.01),
            ("cutoff_cv_amount", -1.01),
        ] {
            assert!(StockVcf::build(&node(&[(name, value)]), 48_000).is_err());
        }
        let (lo, hi) = frequency_bounds();
        assert!(StockVcf::build(&node(&[("frequency", lo)]), 48_000).is_ok());
        assert!(StockVcf::build(&node(&[("frequency", hi)]), 48_000).is_ok());
    }
    #[test]
    fn sdk_rng_seed_warmup_conversion_and_group_cadence() {
        let mut m = model();
        assert_eq!(
            m.draw().to_bits(),
            ((0x7e689c00_u32 as f32) * f32::from_bits(0x2f7fffff)).to_bits()
        );
        let mut a = model();
        let mut b = model();
        step(&mut a, input(5), 3);
        b.draw();
        b.draw();
        assert_eq!(a.rng, b.rng);
    }
    #[test]
    fn raw_audio_has_no_broadcast_and_cv_mono_does() {
        let mut m = model();
        let mut i = input(1);
        i[0] = PortFrame::mono(1.0);
        m.parameters[2] = 1.0;
        step(&mut m, i, 3);
        assert_ne!(m.lanes[0], m.lanes[1]);
        assert_eq!(m.lanes[1], m.lanes[2]);
        assert_eq!(m.lanes[2], m.lanes[3]);
        assert_eq!(m.lanes[4], Lane::default());
    }
    #[test]
    fn rounded_padding_continues_and_whole_groups_freeze() {
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
    }
    #[test]
    fn neither_connected_freezes_filter_and_rng_and_reconnection_resumes() {
        let mut a = model();
        let mut b = model();
        for _ in 0..19 {
            step(&mut a, input(5), 3);
            step(&mut b, input(5), 3);
        }
        for _ in 0..27 {
            let out = step(&mut a, input(16), 0);
            assert_eq!(out[0].values, [0.0; 16]);
        }
        assert_eq!(a.lanes, b.lanes);
        assert_eq!(a.rng, b.rng);
        assert_eq!(step(&mut a, input(5), 3), step(&mut b, input(5), 3));
    }
    #[test]
    fn disconnected_output_decimator_freezes_independently() {
        let mut m = model();
        for _ in 0..17 {
            step(&mut m, input(4), 3);
        }
        let old = m.lanes;
        step(&mut m, input(4), 1);
        for (current, prior) in m.lanes.iter().zip(old).take(4) {
            assert_eq!(current.high, prior.high);
            assert_ne!(current.low, prior.low);
        }
        let old = m.lanes;
        step(&mut m, input(4), 2);
        for (current, prior) in m.lanes.iter().zip(old).take(4) {
            assert_eq!(current.low, prior.low);
            assert_ne!(current.high, prior.high);
        }
    }
    #[test]
    fn adoption_preserves_history_and_rng_only_at_same_rate() {
        let mut old = model();
        for _ in 0..31 {
            step(&mut old, input(16), 3);
        }
        let mut next = StockVcf::build(&node(&[("drive", 1.0)]), 48_000).unwrap();
        next.adopt(&old);
        assert_eq!(next.lanes, old.lanes);
        assert_eq!(next.rng, old.rng);
        assert_eq!(next.parameters[4], 1.0);
        let mut rate = StockVcf::build(&node(&[]), 96_000).unwrap();
        let rng = rate.rng;
        rate.adopt(&old);
        assert_eq!(rate.lanes, [Lane::default(); 16]);
        assert_eq!(rate.rng, rng);
    }
    #[test]
    fn initialize_restores_all_filter_states_and_deterministic_noise() {
        let mut m = model();
        for _ in 0..91 {
            step(&mut m, input(16), 3);
        }
        m.initialize_state();
        let mut fresh = model();
        assert_eq!(m.lanes, fresh.lanes);
        assert_eq!(m.rng, fresh.rng);
        for _ in 0..31 {
            assert_eq!(step(&mut m, input(16), 3), step(&mut fresh, input(16), 3));
        }
    }
    #[test]
    fn nonfinite_inputs_zero_and_host_extremes_remain_finite() {
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
