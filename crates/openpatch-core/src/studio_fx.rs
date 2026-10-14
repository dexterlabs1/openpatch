//! Original bounded studio effects, not emulations of Surge, Plateau, or Pressor.
//! Audio ports use channel one. Delay/reverb gains are independent dry + wet.
use crate::{parameter, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};
use std::f64::consts::TAU;

pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "stereo_delay" | "stereo_reverb" => Some(&["left", "right"]),
        "sidechain_compressor" => Some(&["left", "right", "sidechain"]),
        _ => None,
    }
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "stereo_delay" | "stereo_reverb" => Some(&["left", "right"]),
        "sidechain_compressor" => Some(&["left", "right", "gain"]),
        _ => None,
    }
}
pub(crate) fn catalog() -> Vec<Value> {
    let spec = [
        ("stereo_delay", "STEREO DELAY", vec![
            ("time_left", 0.234375, 0.001, 2.0, "s"), ("time_right", 0.46875, 0.001, 2.0, "s"),
            ("feedback", 0.15, 0.0, 0.95, "x"), ("crossfeed", 0.5, 0.0, 1.0, "x"),
            ("dry", 0.6, 0.0, 2.0, "x"), ("wet", 0.4, 0.0, 2.0, "x"),
            ("low_cut", 300.0, 10.0, 10000.0, "Hz"), ("high_cut", 8000.0, 20.0, 20000.0, "Hz"),
            ("mod_rate", 4.0, 0.0, 20.0, "Hz"), ("mod_depth", 0.0, 0.0, 1.0, "x"),
        ], "Original fractional stereo delay with filtered convex cross-feedback. Modulation depth 1 changes delay by at most 0.5%; no tempo input."),
        ("stereo_reverb", "STEREO REVERB", vec![
            ("dry", 1.0, 0.0, 2.0, "x"), ("wet", 0.35, 0.0, 2.0, "x"),
            ("predelay", 0.01, 0.0, 0.5, "s"), ("size", 0.6, 0.0, 1.0, "x"),
            ("decay", 0.65, 0.0, 1.0, "x"), ("damping", 6000.0, 200.0, 20000.0, "Hz"),
        ], "Original four-line stereo feedback delay network. Decay maps to nominal RT60 = 0.3 + 8*decay^2 seconds; frequency damping shortens the high-frequency tail. Independent unity-capable dry and wet gains."),
        ("sidechain_compressor", "STEREO COMP", vec![
            ("threshold_db", -10.5, -60.0, 12.0, "dB"), ("ratio", 4.0, 1.0, 20.0, "x"),
            ("attack_ms", 2.0, 0.1, 200.0, "ms"), ("release_ms", 200.0, 5.0, 3000.0, "ms"),
            ("knee_db", 6.0, 0.0, 24.0, "dB"), ("makeup_db", 0.0, -24.0, 24.0, "dB"),
            ("detector_ms", 10.0, 0.1, 100.0, "ms"),
        ], "Original stereo-linked RMS soft-knee compressor, 5 V reference. External sidechain channel one overrides the maximum stereo detector. Gain output is 10 V times gain reduction, before makeup."),
    ];
    spec.into_iter().map(|(kind,title,params,description)| {
        let params: serde_json::Map<String,Value> = params.into_iter().map(|(name,default,min,max,unit)|
            (name.to_owned(), json!({"default":default,"min":min,"max":max,"unit":unit}))).collect();
        json!({"kind":kind,"title":title,"model":"functional","inputs":input_names(kind).unwrap(),"outputs":output_names(kind).unwrap(),"params":params,"description":description})
    }).collect()
}

#[derive(Clone, Debug)]
struct Line {
    data: Vec<f64>,
    head: usize,
}
impl Line {
    fn new(size: usize) -> Self {
        Self {
            data: vec![0.0; size.max(3)],
            head: 0,
        }
    }
    fn read(&self, delay: f64) -> f64 {
        let delay = delay.clamp(1.0, (self.data.len() - 2) as f64);
        let whole = delay.floor() as usize;
        let fraction = delay - whole as f64;
        let a = (self.head + self.data.len() - whole) % self.data.len();
        let b = (a + self.data.len() - 1) % self.data.len();
        self.data[a] * (1.0 - fraction) + self.data[b] * fraction
    }
    fn push(&mut self, value: f64) {
        self.data[self.head] = finite(value);
        self.head = (self.head + 1) % self.data.len();
    }
    fn adopt(&mut self, old: &Self) {
        self.data.copy_from_slice(&old.data);
        self.head = old.head;
    }
    fn reset(&mut self) {
        self.data.fill(0.0);
        self.head = 0;
    }
}
fn finite(x: f64) -> f64 {
    if x.is_finite() {
        x.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT))
    } else {
        0.0
    }
}
fn pole(hz: f64, rate: f64) -> f64 {
    1.0 - (-TAU * hz.min(rate * 0.45) / rate).exp()
}
fn sample(inputs: &[PortFrame], index: usize, normal: f64) -> f64 {
    inputs
        .get(index)
        .filter(|f| f.channels > 0)
        .map_or(normal, |f| finite(f64::from(f.values[0])))
}
#[derive(Clone, Debug)]
struct Delay {
    lines: [Line; 2],
    times: [f64; 2],
    feedback: f64,
    crossfeed: f64,
    dry: f64,
    wet: f64,
    hp: f64,
    lp: f64,
    low: [f64; 2],
    high: [f64; 2],
    phase: f64,
    phase_step: f64,
    depth: f64,
}
#[derive(Clone, Debug)]
struct Reverb {
    pre: [Line; 2],
    lines: [Line; 4],
    times: [f64; 4],
    gains: [f64; 4],
    damp: [f64; 4],
    lp: f64,
    predelay: f64,
    dry: f64,
    wet: f64,
}
#[derive(Clone, Debug)]
struct Compressor {
    threshold: f64,
    ratio: f64,
    knee: f64,
    makeup: f64,
    detector: f64,
    attack: f64,
    release: f64,
    power: f64,
    gain: f64,
}
impl Compressor {
    fn target(&self, db: f64) -> f64 {
        let x = db - self.threshold;
        let reduction = if self.knee > 0.0 && x.abs() < self.knee * 0.5 {
            (1.0 - 1.0 / self.ratio) * (x + self.knee * 0.5).powi(2) / (2.0 * self.knee)
        } else {
            (1.0 - 1.0 / self.ratio) * x.max(0.0)
        };
        10.0_f64.powf(-reduction / 20.0)
    }
}
#[derive(Clone, Debug)]
enum Effect {
    Delay(Delay),
    Reverb(Reverb),
    Compressor(Compressor),
}
#[derive(Clone, Debug)]
pub(crate) struct StudioFx {
    rate: u32,
    effect: Effect,
}
impl StudioFx {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if rate == 0 || rate > 384000 {
            return Err("studio effects require a sample rate between 1 and 384000 Hz".into());
        }
        let allowed: &[&str] = match node.kind.as_str() {
            "stereo_delay" => &[
                "time_left",
                "time_right",
                "feedback",
                "crossfeed",
                "dry",
                "wet",
                "low_cut",
                "high_cut",
                "mod_rate",
                "mod_depth",
            ],
            "stereo_reverb" => &["dry", "wet", "predelay", "size", "decay", "damping"],
            "sidechain_compressor" => &[
                "threshold_db",
                "ratio",
                "attack_ms",
                "release_ms",
                "knee_db",
                "makeup_db",
                "detector_ms",
            ],
            _ => return Err(format!("unknown studio effect {}", node.kind)),
        };
        for name in node.params.keys() {
            if !allowed.contains(&name.as_str()) {
                return Err(format!("unknown {} parameter {name}", node.kind));
            }
        }
        let r = f64::from(rate);
        let p = |name, default, min, max| parameter(node, name, default, min, max).map(f64::from);
        let effect = match node.kind.as_str() {
            "stereo_delay" => Effect::Delay(Delay {
                lines: std::array::from_fn(|_| Line::new((2.02 * r).ceil() as usize + 3)),
                times: [
                    p("time_left", 0.234375, 0.001, 2.0)? * r,
                    p("time_right", 0.46875, 0.001, 2.0)? * r,
                ],
                feedback: p("feedback", 0.15, 0.0, 0.95)?,
                crossfeed: p("crossfeed", 0.5, 0.0, 1.0)?,
                dry: p("dry", 0.6, 0.0, 2.0)?,
                wet: p("wet", 0.4, 0.0, 2.0)?,
                hp: pole(p("low_cut", 300.0, 10.0, 10000.0)?, r),
                lp: pole(p("high_cut", 8000.0, 20.0, 20000.0)?, r),
                low: [0.0; 2],
                high: [0.0; 2],
                phase: 0.0,
                phase_step: TAU * p("mod_rate", 4.0, 0.0, 20.0)? / r,
                depth: 0.005 * p("mod_depth", 0.0, 0.0, 1.0)?,
            }),
            "stereo_reverb" => {
                let scale = 0.5 + 1.5 * p("size", 0.6, 0.0, 1.0)?;
                let times = [0.0297, 0.0371, 0.0411, 0.0437].map(|s: f64| (s * scale * r).max(1.0));
                let rt60 = 0.3 + 8.0 * p("decay", 0.65, 0.0, 1.0)?.powi(2);
                Effect::Reverb(Reverb {
                    pre: std::array::from_fn(|_| Line::new((0.5 * r).ceil() as usize + 3)),
                    lines: std::array::from_fn(|_| Line::new((0.1 * r).ceil() as usize + 3)),
                    times,
                    gains: times.map(|t| 10.0_f64.powf(-3.0 * t / (r * rt60))),
                    damp: [0.0; 4],
                    lp: pole(p("damping", 6000.0, 200.0, 20000.0)?, r),
                    predelay: p("predelay", 0.01, 0.0, 0.5)? * r,
                    dry: p("dry", 1.0, 0.0, 2.0)?,
                    wet: p("wet", 0.35, 0.0, 2.0)?,
                })
            }
            "sidechain_compressor" => Effect::Compressor(Compressor {
                threshold: p("threshold_db", -10.5, -60.0, 12.0)?,
                ratio: p("ratio", 4.0, 1.0, 20.0)?,
                knee: p("knee_db", 6.0, 0.0, 24.0)?,
                makeup: 10.0_f64.powf(p("makeup_db", 0.0, -24.0, 24.0)? / 20.0),
                detector: 1.0 - (-1.0 / (r * p("detector_ms", 10.0, 0.1, 100.0)? * 0.001)).exp(),
                attack: 1.0 - (-1.0 / (r * p("attack_ms", 2.0, 0.1, 200.0)? * 0.001)).exp(),
                release: 1.0 - (-1.0 / (r * p("release_ms", 200.0, 5.0, 3000.0)? * 0.001)).exp(),
                power: 0.0,
                gain: 1.0,
            }),
            _ => return Err(format!("unknown studio effect {}", node.kind)),
        };
        Ok(Self { rate, effect })
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let left = sample(inputs, 0, 0.0);
        let input = [left, sample(inputs, 1, left)];
        let (audio, gain) = match &mut self.effect {
            Effect::Delay(d) => {
                let modulation = d.depth * d.phase.sin();
                let delayed = std::array::from_fn::<_, 2, _>(|i| {
                    d.lines[i]
                        .read(d.times[i] * (1.0 + if i == 0 { modulation } else { -modulation }))
                });
                d.phase = (d.phase + d.phase_step) % TAU;
                for (i, &value) in delayed.iter().enumerate() {
                    d.low[i] += d.hp * (value - d.low[i]);
                    d.high[i] += d.lp * (value - d.low[i] - d.high[i]);
                }
                for (i, &value) in input.iter().enumerate() {
                    d.lines[i].push(
                        value
                            + d.feedback
                                * ((1.0 - d.crossfeed) * d.high[i] + d.crossfeed * d.high[1 - i]),
                    );
                }
                (
                    [
                        input[0] * d.dry + delayed[0] * d.wet,
                        input[1] * d.dry + delayed[1] * d.wet,
                    ],
                    0.0,
                )
            }
            Effect::Reverb(d) => {
                let mut pre = input;
                for i in 0..2 {
                    // Sub-sample predelay interpolates the direct signal and one sample of history.
                    pre[i] = if d.predelay < 1.0 {
                        input[i] * (1.0 - d.predelay) + d.pre[i].read(1.0) * d.predelay
                    } else {
                        d.pre[i].read(d.predelay)
                    };
                    d.pre[i].push(input[i]);
                }
                let mut v = [0.0; 4];
                for (i, value) in v.iter_mut().enumerate() {
                    *value = d.lines[i].read(d.times[i]);
                    d.damp[i] += d.lp * (*value - d.damp[i]);
                }
                let [a, b, c, e] = d.damp;
                let mixed = [a + b + c + e, a - b + c - e, a + b - c - e, a - b - c + e];
                let inject = [pre[0], pre[1], pre[0], -pre[1]];
                for i in 0..4 {
                    d.lines[i].push(0.5 * inject[i] + 0.5 * mixed[i] * d.gains[i]);
                }
                (
                    [
                        input[0] * d.dry + (v[0] + v[2]) * 0.5 * d.wet,
                        input[1] * d.dry + (v[1] - v[3]) * 0.5 * d.wet,
                    ],
                    0.0,
                )
            }
            Effect::Compressor(c) => {
                let detect = sample(inputs, 2, input[0].abs().max(input[1].abs())) / 5.0;
                c.power += c.detector * (detect * detect - c.power);
                let target = c.target(10.0 * c.power.max(1e-30).log10());
                c.gain += (if target < c.gain { c.attack } else { c.release }) * (target - c.gain);
                (
                    [input[0] * c.gain * c.makeup, input[1] * c.gain * c.makeup],
                    10.0 * c.gain,
                )
            }
        };
        for (i, out) in outputs.iter_mut().enumerate() {
            *out = PortFrame::mono(finite(if i < 2 {
                audio[i]
            } else if i == 2 {
                gain
            } else {
                0.0
            }) as f32);
        }
    }
    /// Preserve history only across compatible rates/algorithms; new parameters remain authoritative.
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate != old.rate {
            return;
        }
        match (&mut self.effect, &old.effect) {
            (Effect::Delay(a), Effect::Delay(b)) => {
                for i in 0..2 {
                    a.lines[i].adopt(&b.lines[i]);
                }
                a.low = b.low;
                a.high = b.high;
                a.phase = b.phase;
            }
            (Effect::Reverb(a), Effect::Reverb(b)) => {
                for i in 0..2 {
                    a.pre[i].adopt(&b.pre[i]);
                }
                for i in 0..4 {
                    a.lines[i].adopt(&b.lines[i]);
                }
                a.damp = b.damp;
            }
            (Effect::Compressor(a), Effect::Compressor(b)) => {
                a.power = b.power;
                a.gain = b.gain;
            }
            _ => {}
        }
    }
    pub(crate) fn reset_state(&mut self) {
        match &mut self.effect {
            Effect::Delay(d) => {
                for l in &mut d.lines {
                    l.reset();
                }
                d.low = [0.0; 2];
                d.high = [0.0; 2];
                d.phase = 0.0;
            }
            Effect::Reverb(d) => {
                for l in &mut d.lines {
                    l.reset();
                }
                for l in &mut d.pre {
                    l.reset();
                }
                d.damp = [0.0; 4];
            }
            Effect::Compressor(c) => {
                c.power = 0.0;
                c.gain = 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn effect(kind: &str, params: Value, rate: u32) -> StudioFx {
        let node: Node =
            serde_json::from_value(json!({"id":"fx","kind":kind,"params":params})).unwrap();
        StudioFx::build(&node, rate).unwrap()
    }
    fn tick(fx: &mut StudioFx, left: f32, right: Option<f32>, side: Option<f32>) -> [f32; 3] {
        let inputs = [
            PortFrame::mono(left),
            right.map(PortFrame::mono).unwrap_or_default(),
            side.map(PortFrame::mono).unwrap_or_default(),
        ];
        let mut outputs = [PortFrame::default(); 3];
        fx.process(&inputs, &mut outputs);
        assert!(outputs
            .iter()
            .all(|o| o.channels == 1 && o.values.iter().all(|v| v.is_finite())));
        outputs.map(|o| o.values[0])
    }
    #[test]
    fn rejects_unknown_and_invalid_parameters() {
        for kind in ["stereo_delay", "stereo_reverb", "sidechain_compressor"] {
            let node: Node =
                serde_json::from_value(json!({"id":"fx","kind":kind,"params":{"typo":1}})).unwrap();
            assert!(StudioFx::build(&node, 48000).is_err());
        }
        let node: Node = serde_json::from_value(
            json!({"id":"fx","kind":"stereo_delay","params":{"feedback":1}}),
        )
        .unwrap();
        assert!(StudioFx::build(&node, 48000).is_err());
    }
    #[test]
    fn fractional_delay_timing_crossfeed_and_stable_tail() {
        let mut delay = effect(
            "stereo_delay",
            json!({"time_left":0.0105,"time_right":0.02,"dry":0,"wet":1,"feedback":0,"mod_depth":0}),
            1000,
        );
        let trace: Vec<_> = (0..32)
            .map(|n| tick(&mut delay, if n == 0 { 1.0 } else { 0.0 }, Some(0.0), None)[0])
            .collect();
        assert!(trace[..10].iter().all(|v| v.abs() < 1e-6));
        assert!((trace[10] - 0.5).abs() < 1e-5 && (trace[11] - 0.5).abs() < 1e-5);
        assert!(trace[12..].iter().all(|v| v.abs() < 1e-6));
        let mut delay = effect(
            "stereo_delay",
            json!({"time_left":0.01,"time_right":0.02,"dry":0,"wet":1,"feedback":0.5,"crossfeed":1,"low_cut":10,"high_cut":400}),
            1000,
        );
        let mut cross = 0.0_f32;
        let mut late = 0.0_f32;
        for n in 0..10000 {
            let out = tick(&mut delay, if n == 0 { 1.0 } else { 0.0 }, Some(0.0), None);
            if n < 30 {
                assert!(out[1].abs() < 1e-5);
            }
            if (30..60).contains(&n) {
                cross = cross.max(out[1].abs());
            }
            if n > 9000 {
                late = late.max(out[0].abs()).max(out[1].abs());
            }
            assert!(out[0].abs() < 1.01 && out[1].abs() < 1.01);
        }
        assert!(cross > 0.1 && late < 1e-6);
    }
    #[test]
    fn reverb_mono_normal_is_stereo_and_decays_with_unity_dry() {
        let mut reverb = effect(
            "stereo_reverb",
            json!({"dry":1,"wet":1,"predelay":0.01,"decay":0.3}),
            8000,
        );
        let mut early = 0.0_f64;
        let mut late = 0.0_f64;
        let mut difference = 0.0_f64;
        for n in 0..64000 {
            let out = tick(&mut reverb, if n == 0 { 1.0 } else { 0.0 }, None, None);
            if n == 0 {
                assert_eq!(&out[..2], &[1.0, 1.0]);
            }
            if n > 0 && n < 80 {
                assert_eq!(&out[..2], &[0.0, 0.0]);
            }
            if (80..8000).contains(&n) {
                early += f64::from(out[0] * out[0] + out[1] * out[1]);
                difference += f64::from((out[0] - out[1]).abs());
            }
            if n > 56000 {
                late += f64::from(out[0] * out[0] + out[1] * out[1]);
            }
        }
        assert!(early > 0.01 && difference > 0.1);
        assert!(late < early * 1e-6);
    }
    #[test]
    fn compressor_calibrated_ratio_knee_stereo_link_and_release() {
        let mut fx = effect(
            "sidechain_compressor",
            json!({"threshold_db":-12,"ratio":4,"knee_db":0,"attack_ms":1,"release_ms":50,"detector_ms":1}),
            8000,
        );
        let mut out = [0.0; 3];
        for _ in 0..8000 {
            out = tick(&mut fx, 5.0, Some(-2.5), Some(5.0));
        }
        let expected = 10.0_f32.powf(-9.0 / 20.0);
        assert!((out[0] / 5.0 - expected).abs() < 1e-5);
        assert!((out[1] / -2.5 - expected).abs() < 1e-5);
        assert!((out[2] / 10.0 - expected).abs() < 1e-5);
        for _ in 0..8000 {
            out = tick(&mut fx, 5.0, Some(-2.5), Some(0.0));
        }
        assert!((out[0] - 5.0).abs() < 1e-4); // Patched zero sidechain overrides loud program audio.
        let mut normal = effect(
            "sidechain_compressor",
            json!({"threshold_db":-12,"ratio":4,"knee_db":0}),
            8000,
        );
        for _ in 0..8000 {
            out = tick(&mut normal, 2.5, Some(5.0), None);
        }
        assert!((out[1] / 5.0 - expected).abs() < 1e-5); // Right channel participates in linked detector.
        let hard = Compressor {
            threshold: -12.0,
            ratio: 4.0,
            knee: 0.0,
            makeup: 1.0,
            detector: 0.0,
            attack: 0.0,
            release: 0.0,
            power: 0.0,
            gain: 1.0,
        };
        let soft = Compressor {
            knee: 6.0,
            ..hard.clone()
        };
        assert_eq!(soft.target(-15.0), 1.0);
        assert!(soft.target(-12.0) < hard.target(-12.0));
        assert!((soft.target(-9.0) - hard.target(-9.0)).abs() < 1e-12);
        assert!(soft.target(-12.001) > soft.target(-11.999));
    }
    #[test]
    fn adoption_preserves_histories_without_replacing_allocations_or_parameters() {
        for kind in ["stereo_delay", "stereo_reverb", "sidechain_compressor"] {
            for rate in [8000, 44100, 48000, 96000, 192000] {
                let mut old = effect(kind, json!({}), rate);
                for n in 0..5000 {
                    tick(&mut old, if n % 73 == 0 { 1.0 } else { 0.0 }, None, None);
                }
                let mut new = effect(kind, json!({}), rate);
                let addresses = |fx: &StudioFx| -> Vec<usize> {
                    match &fx.effect {
                        Effect::Delay(d) => {
                            d.lines.iter().map(|l| l.data.as_ptr() as usize).collect()
                        }
                        Effect::Reverb(d) => d
                            .lines
                            .iter()
                            .chain(d.pre.iter())
                            .map(|l| l.data.as_ptr() as usize)
                            .collect(),
                        Effect::Compressor(_) => vec![],
                    }
                };
                let before = addresses(&new);
                new.adopt(&old);
                assert_eq!(addresses(&new), before);
                for _ in 0..100 {
                    assert_eq!(
                        tick(&mut old, 0.1, None, None),
                        tick(&mut new, 0.1, None, None)
                    );
                }
                new.reset_state();
                let mut fresh = effect(kind, json!({}), rate);
                assert_eq!(
                    tick(&mut new, 0.0, None, None),
                    tick(&mut fresh, 0.0, None, None)
                );
                let mut changed = effect(
                    kind,
                    if kind == "sidechain_compressor" {
                        json!({"makeup_db":6})
                    } else {
                        json!({"dry":0})
                    },
                    rate,
                );
                changed.adopt(&old);
                match &changed.effect {
                    Effect::Delay(d) => assert_eq!(d.dry, 0.0),
                    Effect::Reverb(d) => assert_eq!(d.dry, 0.0),
                    Effect::Compressor(c) => assert!(c.makeup > 1.9),
                }
                let mut other_rate = effect(kind, json!({}), rate + 1);
                other_rate.adopt(&old);
                let mut other_fresh = effect(kind, json!({}), rate + 1);
                assert_eq!(
                    tick(&mut other_rate, 0.0, None, None),
                    tick(&mut other_fresh, 0.0, None, None)
                );
            }
        }
    }
    #[test]
    fn hostile_input_and_maximum_feedback_remain_bounded() {
        for kind in ["stereo_delay", "stereo_reverb", "sidechain_compressor"] {
            let mut fx = effect(
                kind,
                match kind {
                    "stereo_delay" => {
                        json!({"feedback":0.95,"mod_depth":1,"time_left":0.001,"time_right":0.002})
                    }
                    "stereo_reverb" => json!({"decay":1}),
                    _ => json!({}),
                },
                8000,
            );
            for n in 0..16000 {
                let out = tick(
                    &mut fx,
                    match n % 4 {
                        0 => f32::NAN,
                        1 => f32::INFINITY,
                        2 => SIGNAL_LIMIT,
                        _ => -SIGNAL_LIMIT,
                    },
                    None,
                    None,
                );
                assert!(out.iter().all(|v| v.abs() <= SIGNAL_LIMIT));
            }
        }
    }
}
