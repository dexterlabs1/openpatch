//! Original character effects. All buffers are allocated at graph construction.
use super::{pole, sample, Line};
use crate::{parameter, Node, PortFrame};
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};
pub(super) fn supports(kind: &str) -> bool {
    matches!(
        kind,
        "diffuse_reverb" | "tape_delay" | "stereo_ensemble" | "bass_glitch"
    )
}
pub(super) const INPUTS: [&str; 5] = ["left", "right", "amount_cv", "time_cv", "freeze"];
pub(super) fn catalog() -> Vec<Value> {
    let all=[
 ("diffuse_reverb","DIFFUSE SPACE","Dense modulated eight-line orthogonal feedback reverb with four input allpass diffusers per side, predelay, low cut, damping, stereo width and freeze. Original OpenPatch algorithm.",vec![("size",0.75,0.,1.),("decay",0.72,0.,1.),("damping",6500.,200.,20000.),("predelay",0.025,0.,0.5),("diffusion",0.75,0.,1.),("motion",0.5,0.,1.),("low_cut",130.,20.,2000.),("width",1.2,0.,2.),("dry",1.,0.,2.),("wet",0.4,0.,2.)]),
 ("tape_delay","TAPE ECHO","Smoothed fractional stereo tape echo with softly saturated feedback, filtered repeats, wow/flutter and stereo crossfeed. Time CV adds +/- one octave of delay time. Original OpenPatch algorithm.",vec![("time_left",0.375,0.01,2.),("time_right",0.5,0.01,2.),("feedback",0.45,0.,0.92),("drive",0.35,0.,1.),("wow",0.35,0.,1.),("crossfeed",0.75,0.,1.),("low_cut",180.,20.,2000.),("high_cut",4500.,400.,16000.),("dry",1.,0.,2.),("wet",0.35,0.,2.)]),
 ("stereo_ensemble","ENSEMBLE","Six modulated fractional delay voices with independent stereo phase offsets and a high-passed wet path, preserving the dry low fundamental. Original OpenPatch algorithm.",vec![("rate",0.18,0.01,4.),("depth",0.65,0.,1.),("width",1.25,0.,2.),("low_cut",180.,20.,2000.),("dry",0.85,0.,2.),("wet",0.4,0.,2.)]),
 ("bass_glitch","BASS GLITCH","Tunable granular stutter: captured loops, repeats, reverse, pitch, sample-rate reduction, bit crushing, filtered saturated feedback and freeze. Smooth two-head windows reduce seam clicks. Low cut removes wet sub energy so a parallel clean sub can retain weight. Original OpenPatch algorithm.",vec![("grain_ms",125.,15.,500.),("repeats",4.,1.,16.),("pitch",-5.,-12.,12.),("reverse",0.,0.,1.),("bits",12.,4.,16.),("reduction",1.,1.,32.),("feedback",0.25,0.,0.85),("drive",0.25,0.,1.),("low_cut",120.,20.,1000.),("high_cut",5000.,400.,16000.),("freeze",0.,0.,1.),("mix",0.4,0.,1.)]),
 ];
    all.into_iter().map(|(kind,title,description,params)|{let mut spec=serde_json::Map::new();for (k,d,lo,hi) in params{let mut p=json!({"default":d,"min":lo,"max":hi});if ["repeats","reverse","bits","reduction","freeze"].contains(&k){p["step"]=json!(1);}if ["low_cut","high_cut","damping"].contains(&k){p["unit"]=json!("Hz");}if k.starts_with("time_")||k=="predelay"{p["unit"]=json!("s");}if k=="grain_ms"{p["unit"]=json!("ms");}spec.insert(k.into(),p);}json!({"kind":kind,"title":title,"model":"functional","description":description,"inputs":INPUTS.to_vec(),"params":spec})}).collect()
}
#[derive(Clone, Debug)]
struct Space {
    pre: [Line; 2],
    diffuse: [[Line; 4]; 2],
    lines: [Line; 8],
    damp: [f64; 8],
    phase: f64,
    low: [f64; 2],
}
#[derive(Clone, Debug)]
struct Tape {
    lines: [Line; 2],
    times: [f64; 2],
    low: [f64; 2],
    high: [f64; 2],
    phase: f64,
}
#[derive(Clone, Debug)]
struct Ensemble {
    lines: [Line; 2],
    low: [f64; 2],
    phase: f64,
}
#[derive(Clone, Debug)]
struct Glitch {
    live: [Vec<f64>; 2],
    held: [Vec<f64>; 2],
    write: usize,
    origin: usize,
    filled: usize,
    phase: f64,
    count: usize,
    low: [f64; 2],
    high: [f64; 2],
    last: [f64; 2],
    quantized: [f64; 2],
    tick: usize,
    window: usize,
    mix: f64,
}
#[derive(Clone, Debug)]
enum State {
    Space(Space),
    Tape(Tape),
    Ensemble(Ensemble),
    Glitch(Glitch),
}
#[derive(Clone, Debug)]
pub(super) struct CreativeFx {
    rate: f64,
    params: [f64; 12],
    state: State,
}
impl CreativeFx {
    pub(super) fn build(n: &Node, rate: u32) -> Result<Self, String> {
        let schema = catalog()
            .into_iter()
            .find(|s| s["kind"] == n.kind)
            .ok_or("Unknown creative effect")?;
        for k in n.params.keys() {
            if schema["params"].get(k).is_none() {
                return Err(format!("unknown {} parameter {k}", n.kind));
            }
        }
        let p = |key: &str| {
            let s = &schema["params"][key];
            parameter(
                n,
                key,
                s["default"].as_f64().unwrap() as f32,
                s["min"].as_f64().unwrap() as f32,
                s["max"].as_f64().unwrap() as f32,
            )
            .map(f64::from)
        };
        let r = f64::from(rate);
        let mut params = [0.; 12];
        let (names, state): (&[&str], State) = match n.kind.as_str() {
            "diffuse_reverb" => (
                &[
                    "size",
                    "decay",
                    "damping",
                    "predelay",
                    "diffusion",
                    "motion",
                    "low_cut",
                    "width",
                    "dry",
                    "wet",
                ],
                State::Space(Space {
                    pre: std::array::from_fn(|_| Line::new((r * 0.5) as usize + 3)),
                    diffuse: std::array::from_fn(|_| {
                        std::array::from_fn(|_| Line::new((r * 0.035) as usize + 3))
                    }),
                    lines: std::array::from_fn(|_| Line::new((r * 0.3) as usize + 3)),
                    damp: [0.; 8],
                    phase: 0.,
                    low: [0.; 2],
                }),
            ),
            "tape_delay" => (
                &[
                    "time_left",
                    "time_right",
                    "feedback",
                    "drive",
                    "wow",
                    "crossfeed",
                    "low_cut",
                    "high_cut",
                    "dry",
                    "wet",
                ],
                State::Tape(Tape {
                    lines: std::array::from_fn(|_| Line::new((r * 2.05) as usize + 3)),
                    times: [p("time_left")? * r, p("time_right")? * r],
                    low: [0.; 2],
                    high: [0.; 2],
                    phase: 0.,
                }),
            ),
            "stereo_ensemble" => (
                &["rate", "depth", "width", "low_cut", "dry", "wet"],
                State::Ensemble(Ensemble {
                    lines: std::array::from_fn(|_| Line::new((r * 0.07) as usize + 3)),
                    low: [0.; 2],
                    phase: 0.,
                }),
            ),
            "bass_glitch" => (
                &[
                    "grain_ms",
                    "repeats",
                    "pitch",
                    "reverse",
                    "bits",
                    "reduction",
                    "feedback",
                    "drive",
                    "low_cut",
                    "high_cut",
                    "freeze",
                    "mix",
                ],
                State::Glitch(Glitch {
                    live: std::array::from_fn(|_| vec![0.; (r * 0.5).ceil() as usize + 3]),
                    held: std::array::from_fn(|_| vec![0.; (r * 0.5).ceil() as usize + 3]),
                    write: 0,
                    origin: 0,
                    filled: 0,
                    phase: 0.,
                    count: 0,
                    low: [0.; 2],
                    high: [0.; 2],
                    last: [0.; 2],
                    quantized: [0.; 2],
                    tick: 0,
                    window: (p("grain_ms")? * r * 0.001) as usize,
                    mix: p("mix")?,
                }),
            ),
            _ => return Err("Unknown creative effect".into()),
        };
        for (i, key) in names.iter().enumerate() {
            params[i] = p(key)?;
            if schema["params"][key]["step"] == 1 && params[i].fract() != 0. {
                return Err(format!("{key} must be an integer"));
            }
        }
        Ok(Self {
            rate: r,
            params,
            state,
        })
    }
    pub(super) fn process(&mut self, inputs: &[PortFrame]) -> [f64; 2] {
        let l = sample(inputs, 0, 0.);
        let input = [l, sample(inputs, 1, l)];
        let amount = (sample(inputs, 2, 10.) / 10.).clamp(0., 1.);
        let time = sample(inputs, 3, 0.).clamp(-1., 1.);
        let freeze = sample(inputs, 4, 0.) >= 1.;
        let p = self.params;
        let r = self.rate;
        match &mut self.state {
            State::Space(s) => {
                let mut pre = input;
                for ch in 0..2 {
                    pre[ch] = if p[3] * r < 1. {
                        input[ch]
                    } else {
                        s.pre[ch].read(p[3] * r)
                    };
                    s.pre[ch].push(input[ch]);
                    s.low[ch] += pole(p[6], r) * (pre[ch] - s.low[ch]);
                    pre[ch] -= s.low[ch];
                    for j in 0..4 {
                        let delay =
                            [0.0047, 0.0083, 0.0127, 0.0191][j] * r * (1. + ch as f64 * 0.063);
                        let d = s.diffuse[ch][j].read(delay);
                        let g = 0.45 + p[4] * 0.28;
                        let y = d - g * pre[ch];
                        s.diffuse[ch][j].push(pre[ch] + g * y);
                        pre[ch] = y;
                    }
                }
                let times = [
                    0.0533, 0.0611, 0.0713, 0.0799, 0.0893, 0.1013, 0.1139, 0.1277,
                ];
                let size = (0.5 + 1.4 * p[0]) * time.exp2();
                let size = size.clamp(0.45, 2.1);
                let mut v = [0.; 8];
                for i in 0..8 {
                    let modulation = (s.phase * (1. + i as f64 * 0.071) + i as f64 * 1.7).sin()
                        * r
                        * 0.0008
                        * p[5];
                    v[i] = s.lines[i].read(times[i] * size * r + modulation);
                    s.damp[i] += pole(p[2], r) * (v[i] - s.damp[i]);
                }
                let sum = s.damp.iter().sum::<f64>() * 0.25;
                let rt60 = 0.6 + 14. * p[1] * p[1];
                for i in 0..8 {
                    let gain = if freeze {
                        0.9996
                    } else {
                        10f64.powf(-3. * times[i] * size / rt60)
                    };
                    let inject = if freeze {
                        0.
                    } else {
                        pre[i % 2] * if i % 4 < 2 { 0.28 } else { -0.28 }
                    };
                    s.lines[i].push(inject + (s.damp[i] - sum) * gain);
                }
                s.phase = (s.phase + TAU * 0.13 / r) % TAU;
                let wet = [
                    (v[0] + v[2] - v[4] + v[6]) * 0.32,
                    (v[1] - v[3] + v[5] + v[7]) * 0.32,
                ];
                let mid = (wet[0] + wet[1]) * 0.5;
                let side = (wet[0] - wet[1]) * 0.5 * p[7];
                [
                    input[0] * p[8] + (mid + side) * p[9] * amount,
                    input[1] * p[8] + (mid - side) * p[9] * amount,
                ]
            }
            State::Tape(s) => {
                let mut d = [0.; 2];
                for i in 0..2 {
                    let target = (p[i] * time.exp2() * r).clamp(r * 0.01, r * 2.);
                    s.times[i] += (target - s.times[i]) * (1. - (-1. / (r * 0.04)).exp());
                    let wobble = (s.phase + i as f64 * 1.3).sin() * 0.002
                        + (s.phase * 7.13 + i as f64).sin() * 0.0003;
                    d[i] = s.lines[i].read(s.times[i] * (1. + wobble * p[4]));
                    s.low[i] += pole(p[6], r) * (d[i] - s.low[i]);
                    s.high[i] += pole(p[7], r) * (d[i] - s.low[i] - s.high[i]);
                }
                for i in 0..2 {
                    let fb = s.high[i] * (1. - p[5]) + s.high[1 - i] * p[5];
                    let drive = 1. + p[3] * 4.;
                    let v = if freeze {
                        fb * 0.995
                    } else {
                        input[i] + p[2] * fb
                    };
                    s.lines[i].push((v / 5. * drive).tanh() * 5. / drive);
                }
                s.phase = (s.phase + TAU * 0.37 / r) % TAU;
                [
                    input[0] * p[8] + d[0] * p[9] * amount,
                    input[1] * p[8] + d[1] * p[9] * amount,
                ]
            }
            State::Ensemble(s) => {
                let mut wet = [0.; 2];
                for ch in 0..2 {
                    for j in 0..3 {
                        let phase = s.phase + j as f64 * TAU / 3. + ch as f64 * 1.37;
                        let delay = r * (0.019 + 0.009 * p[1] * phase.sin() + j as f64 * 0.002);
                        wet[ch] += s.lines[ch].read(delay) / 3.;
                    }
                    s.lines[ch].push(input[ch]);
                    s.low[ch] += pole(p[3], r) * (wet[ch] - s.low[ch]);
                    wet[ch] -= s.low[ch];
                }
                s.phase = (s.phase + TAU * p[0] * time.exp2() / r) % TAU;
                let mid = (wet[0] + wet[1]) * 0.5;
                let side = (wet[0] - wet[1]) * 0.5 * p[2];
                [
                    input[0] * p[4] + (mid + side) * p[5] * amount,
                    input[1] * p[4] + (mid - side) * p[5] * amount,
                ]
            }
            State::Glitch(s) => {
                let n = (p[0] * 0.001 * r * time.exp2()).clamp(r * 0.015, r * 0.5) as usize;
                if n != s.window {
                    s.window = n;
                    s.write = 0;
                    s.filled = 0;
                    s.phase = 0.;
                    s.count = 0;
                }
                let held = freeze || p[10] > 0.5;
                for ch in 0..2 {
                    s.low[ch] += pole(p[8], r) * (input[ch] - s.low[ch]);
                    let v = input[ch] - s.low[ch] + s.last[ch] * p[6];
                    let drive = 1. + p[7] * 6.;
                    s.live[ch][s.write] = (v * drive / 5.).tanh() * 5. / drive;
                }
                s.write = (s.write + 1) % n;
                s.filled = (s.filled + 1).min(n);
                if s.count == 0 && s.filled == n && !held {
                    std::mem::swap(&mut s.live, &mut s.held);
                    s.origin = s.write;
                    s.count = p[1] as usize;
                }
                let mut wet = [0.; 2];
                for head in 0..2 {
                    let phase = (s.phase + head as f64 * 0.5).fract();
                    let w = (PI * phase).sin().powi(2);
                    let position = (s.origin as f64
                        + if p[3] > 0.5 {
                            (1. - phase) * n as f64
                        } else {
                            phase * n as f64
                        })
                        % n as f64;
                    let a = position as usize;
                    let f = position.fract();
                    for ch in 0..2 {
                        wet[ch] += (s.held[ch][a] * (1. - f) + s.held[ch][(a + 1) % n] * f) * w;
                    }
                }
                s.phase += 2f64.powf(p[2] / 12.) / n as f64;
                if s.phase >= 1. {
                    s.phase -= 1.;
                    s.count = s.count.saturating_sub(1);
                }
                if s.tick == 0 {
                    let q = 2f64.powf(p[4] - 1.);
                    for ch in 0..2 {
                        s.quantized[ch] = (wet[ch] / 5. * q).round() / q * 5.;
                    }
                }
                s.tick = (s.tick + 1) % (p[5] as usize).max(1);
                s.mix += (p[11] * amount - s.mix) * (1. - (-1. / (0.005 * r)).exp());
                let mut output = [0.; 2];
                for ch in 0..2 {
                    s.high[ch] += pole(p[9], r) * (s.quantized[ch] - s.high[ch]);
                    s.last[ch] = s.high[ch];
                    output[ch] = input[ch] * (1. - s.mix) + s.high[ch] * s.mix;
                }
                output
            }
        }
    }
    pub(super) fn adopt(&mut self, old: &Self) {
        if self.rate != old.rate {
            return;
        }
        match (&mut self.state, &old.state) {
            (State::Space(a), State::Space(b)) => {
                for i in 0..2 {
                    a.pre[i].adopt(&b.pre[i]);
                    for j in 0..4 {
                        a.diffuse[i][j].adopt(&b.diffuse[i][j]);
                    }
                }
                for i in 0..8 {
                    a.lines[i].adopt(&b.lines[i]);
                }
                a.damp = b.damp;
                a.low = b.low;
                a.phase = b.phase;
            }
            (State::Tape(a), State::Tape(b)) => {
                for i in 0..2 {
                    a.lines[i].adopt(&b.lines[i]);
                }
                a.times = b.times;
                a.low = b.low;
                a.high = b.high;
                a.phase = b.phase;
            }
            (State::Ensemble(a), State::Ensemble(b)) => {
                for i in 0..2 {
                    a.lines[i].adopt(&b.lines[i]);
                }
                a.low = b.low;
                a.phase = b.phase;
            }
            (State::Glitch(a), State::Glitch(b)) => {
                for i in 0..2 {
                    a.live[i].copy_from_slice(&b.live[i]);
                    a.held[i].copy_from_slice(&b.held[i]);
                }
                a.write = b.write;
                a.origin = b.origin;
                a.filled = b.filled;
                a.phase = b.phase;
                a.count = b.count;
                a.low = b.low;
                a.high = b.high;
                a.last = b.last;
                a.quantized = b.quantized;
                a.tick = b.tick;
                a.window = b.window;
                a.mix = b.mix;
            }
            _ => {}
        }
    }
    pub(super) fn reset(&mut self) {
        match &mut self.state {
            State::Space(s) => {
                for l in &mut s.pre {
                    l.reset();
                }
                for side in &mut s.diffuse {
                    for l in side {
                        l.reset();
                    }
                }
                for l in &mut s.lines {
                    l.reset();
                }
                s.damp = [0.; 8];
                s.low = [0.; 2];
                s.phase = 0.;
            }
            State::Tape(s) => {
                for l in &mut s.lines {
                    l.reset();
                }
                s.low = [0.; 2];
                s.high = [0.; 2];
                s.phase = 0.;
            }
            State::Ensemble(s) => {
                for l in &mut s.lines {
                    l.reset();
                }
                s.low = [0.; 2];
                s.phase = 0.;
            }
            State::Glitch(s) => {
                for x in &mut s.live {
                    x.fill(0.);
                }
                for x in &mut s.held {
                    x.fill(0.);
                }
                s.write = 0;
                s.origin = 0;
                s.filled = 0;
                s.phase = 0.;
                s.count = 0;
                s.low = [0.; 2];
                s.high = [0.; 2];
                s.last = [0.; 2];
                s.quantized = [0.; 2];
                s.tick = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fx(kind: &str, params: Value) -> CreativeFx {
        CreativeFx::build(
            &serde_json::from_value(json!({"id":"fx","kind":kind,"params":params})).unwrap(),
            48000,
        )
        .unwrap()
    }
    #[test]
    fn diffuse_space_has_dense_decorrelated_tail_and_reset_is_silent() {
        let mut f = fx("diffuse_reverb", json!({"dry":0,"wet":1,"decay":0.65}));
        let mut late = 0.;
        let mut diff = 0.;
        let mut hits = 0;
        for i in 0..96000 {
            let x = if i == 0 { 1. } else { 0. };
            let out = f.process(&[PortFrame::mono(x)]);
            assert!(out.iter().all(|x| x.is_finite() && x.abs() < 2.));
            if i > 24000 {
                late += out[0] * out[0] + out[1] * out[1];
                diff += (out[0] - out[1]).powi(2);
                if out[0].abs() > 1e-7 {
                    hits += 1;
                }
            }
        }
        assert!(late > 1e-7 && diff > 1e-7);
        assert!(hits > 50000, "tail should be dense: {hits}");
        f.reset();
        for _ in 0..4096 {
            assert_eq!(f.process(&[]), [0., 0.]);
        }
    }
    #[test]
    fn all_character_effects_adopt_without_changing_continuation() {
        for kind in [
            "diffuse_reverb",
            "tape_delay",
            "stereo_ensemble",
            "bass_glitch",
        ] {
            let mut a = fx(kind, json!({}));
            let mut b = fx(kind, json!({}));
            for i in 0..16000 {
                a.process(&[PortFrame::mono((i as f32 * 0.07).sin())]);
            }
            b.adopt(&a);
            for _ in 0..128 {
                assert_eq!(a.process(&[]), b.process(&[]), "{kind}");
            }
        }
    }
    #[test]
    fn glitch_controls_make_an_audible_transformation_and_mix_zero_is_dry() {
        let mut dry = fx("bass_glitch", json!({"mix":0}));
        let mut wet = fx(
            "bass_glitch",
            json!({"mix":1,"bits":6,"reduction":8,"reverse":1,"pitch":-7}),
        );
        let mut difference = 0.;
        let mut energy = 0.;
        for i in 0..30000 {
            let x = (i as f32 * 0.037).sin();
            let input = [PortFrame::mono(x)];
            assert!((dry.process(&input)[0] - f64::from(x)).abs() < 1e-9);
            let y = wet.process(&input)[0];
            assert!(y.is_finite() && y.abs() < 8.);
            if i > 12000 {
                difference += (y - f64::from(x)).abs();
                energy += y * y;
            }
        }
        assert!(difference > 100. && energy > 10.);
    }
}
