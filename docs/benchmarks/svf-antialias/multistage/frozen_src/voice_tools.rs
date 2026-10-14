//! Original reconstruction tools. These are intentionally not Kickall, XOX,
//! or Surge emulations. Runtime storage is fixed and processing allocates nothing.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS};
use crate::svf_resampler::{Oversampler, LATENCY_SAMPLES};
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};

pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "drum_voice" => Some(&["trigger", "accent"]),
        "svf_filter" => Some(&["signal", "cutoff_cv"]),
        _ => None,
    }
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "drum_voice" => Some(&["out"]),
        "svf_filter" => Some(&["low", "band", "high"]),
        _ => None,
    }
}
pub(crate) fn catalog() -> Vec<Value> {
    vec![
        json!({"kind":"drum_voice","title":"DRUM VOICE","model":"functional","inputs":input_names("drum_voice").unwrap(),"description":"Original mono kick, noise clap, or metallic closed hat. Schmitt trigger, deterministic noise, exponential envelope; not source-plugin DSP parity. Accent normal10V, clamped0..10V.","params":{
        "mode":{"default":0.0,"min":0.0,"max":2.0,"step":1.0},"pitch":{"default":48.0,"min":20.0,"max":4000.0,"unit":"Hz"},"decay":{"default":0.2,"min":0.005,"max":4.0,"unit":"s"},"level":{"default":1.0,"min":0.0,"max":2.0},"tone":{"default":0.5,"min":0.0,"max":1.0},"drive":{"default":0.0,"min":0.0,"max":1.0}}}),
        json!({"kind":"svf_filter","title":"SVF FILTER","model":"functional","inputs":input_names("svf_filter").unwrap(),"description":"Original topology-preserving state-variable low/band/high filter. Signal determines up to16 voices; mono CV broadcasts. Cutoff clamped below Nyquist, finite input sanitization and bounded drive.","params":{
        "cutoff":{"default":1000.0,"min":5.0,"max":20000.0,"unit":"Hz"},"resonance":{"default":0.2,"min":0.0,"max":1.0},"drive":{"default":0.0,"min":0.0,"max":1.0},"cutoff_cv_amount":{"default":0.0,"min":-4.0,"max":4.0,"unit":"oct/V"}}}),
    ]
}
fn finite(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v)
    } else {
        0.0
    }
}
fn input(inputs: &[PortFrame], index: usize) -> PortFrame {
    inputs.get(index).copied().unwrap_or_default()
}
#[derive(Clone, Debug)]
pub(crate) struct Drum {
    rate: f64,
    mode: u8,
    pitch: f64,
    decay: f64,
    level: f64,
    tone: f64,
    drive: f64,
    high: bool,
    age: u64,
    active: bool,
    phase: f64,
    metal: [f64; 6],
    noise: u32,
    low: f64,
    accent: f64,
}
#[derive(Clone, Debug)]
struct DriveLane {
    oversampler: Oversampler,
    cv: [f64; LATENCY_SAMPLES],
    head: usize,
}
impl DriveLane {
    fn new() -> Self { Self { oversampler: Oversampler::new(), cv: [0.0; LATENCY_SAMPLES], head: 0 } }
    fn process(&mut self, x: f64, cv: f64, gain: f64, blend: f64) -> (f64, f64) {
        let delayed_cv = self.cv[self.head]; self.cv[self.head] = cv;
        self.head = (self.head+1)%LATENCY_SAMPLES;
        let denom = gain.tanh();
        (self.oversampler.process_pair(x, 0.0, |x, _| x + blend * (5.0 * (x*gain/5.0).tanh()/denom - x)), delayed_cv)
    }
    fn copy_state_from(&mut self, old: &Self) {
        self.oversampler.copy_state_from(&old.oversampler); self.cv=old.cv; self.head=old.head;
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Filter {
    rate: f64,
    cutoff: f64,
    resonance: f64,
    drive: f64,
    cv: f64,
    states: [[f64; 2]; MAX_CHANNELS],
    drive_lanes: Box<[DriveLane]>,
    active: usize,
}
#[derive(Clone, Debug)]
pub(crate) enum VoiceTools {
    Drum(Drum),
    Filter(Filter),
}
impl VoiceTools {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if rate == 0 {
            return Err("sample rate must be positive".into());
        }
        let allowed: &[&str] = match node.kind.as_str() {
            "drum_voice" => &["mode", "pitch", "decay", "level", "tone", "drive"],
            "svf_filter" => &["cutoff", "resonance", "drive", "cutoff_cv_amount"],
            _ => return Err(format!("unknown voice tool {}", node.kind)),
        };
        if let Some(key) = node
            .params
            .keys()
            .find(|key| !allowed.contains(&key.as_str()))
        {
            return Err(format!("node {}: unknown parameter {}", node.id, key));
        }
        let p = |name, default, min, max| parameter(node, name, default, min, max).map(f64::from);
        match node.kind.as_str() {
            "drum_voice" => {
                let mode = p("mode", 0.0, 0.0, 2.0)?;
                if mode.fract() != 0.0 {
                    return Err("drum mode must be an integer".into());
                }
                Ok(Self::Drum(Drum {
                    rate: rate.into(),
                    mode: mode as u8,
                    pitch: p("pitch", 48.0, 20.0, 4000.0)?,
                    decay: p("decay", 0.2, 0.005, 4.0)?,
                    level: p("level", 1.0, 0.0, 2.0)?,
                    tone: p("tone", 0.5, 0.0, 1.0)?,
                    drive: p("drive", 0.0, 0.0, 1.0)?,
                    high: false,
                    age: 0,
                    active: false,
                    phase: 0.0,
                    metal: [0.0; 6],
                    noise: 0x92d68ca2,
                    low: 0.0,
                    accent: 1.0,
                }))
            }
            "svf_filter" => Ok(Self::Filter(Filter {
                rate: rate.into(),
                cutoff: p("cutoff", 1000.0, 5.0, 20000.0)?,
                resonance: p("resonance", 0.2, 0.0, 1.0)?,
                drive: p("drive", 0.0, 0.0, 1.0)?,
                cv: p("cutoff_cv_amount", 0.0, -4.0, 4.0)?,
                states: [[0.0; 2]; MAX_CHANNELS],
                drive_lanes: (0..MAX_CHANNELS).map(|_| DriveLane::new()).collect(),
                active: 0,
            })),
            _ => Err(format!("unknown voice tool {}", node.kind)),
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        outputs.fill(PortFrame::default());
        match self {
            Self::Drum(d) => {
                let trigger = finite(input(inputs, 0).channel(0));
                let rising = if d.high {
                    if trigger <= 0.1 {
                        d.high = false;
                    }
                    false
                } else if trigger >= 1.0 {
                    d.high = true;
                    true
                } else {
                    false
                };
                if rising {
                    d.age = 0;
                    d.active = true;
                    d.phase = 0.0;
                    d.metal = [0.0; 6];
                    d.accent = (finite(input(inputs, 1).voice(0, 10.0)) / 10.0).clamp(0.0, 1.0);
                }
                let mut value = 0.0;
                if d.active {
                    let time = d.age as f64 / d.rate;
                    let env = (-time * 6.907755278982137 / d.decay).exp();
                    if env < 1e-7 {
                        d.active = false;
                    } else {
                        d.noise ^= d.noise << 13;
                        d.noise ^= d.noise >> 17;
                        d.noise ^= d.noise << 5;
                        let noise = f64::from(d.noise) / f64::from(u32::MAX) * 2.0 - 1.0;
                        let alpha = 1.0
                            - (-TAU * (500.0 + 11000.0 * d.tone).min(d.rate * 0.4) / d.rate).exp();
                        d.low += alpha * (noise - d.low);
                        let raw = match d.mode {
                            0 => {
                                let freq = (d.pitch * (1.0 + 4.0 * d.tone * (-time / 0.025).exp()))
                                    .min(d.rate * 0.4);
                                d.phase = (d.phase + freq / d.rate).fract();
                                (TAU * d.phase).sin()
                            }
                            1 => {
                                let burst = if time < 0.035 {
                                    let local = time % 0.012;
                                    (-local / 0.002).exp()
                                } else {
                                    0.25
                                };
                                d.low * burst
                            }
                            _ => {
                                let mut sum = 0.0;
                                for (phase, ratio) in d
                                    .metal
                                    .iter_mut()
                                    .zip([1.0, 1.342, 1.718, 2.113, 2.731, 3.127])
                                {
                                    *phase = (*phase
                                        + (d.pitch * ratio).min(d.rate * 0.4) / d.rate)
                                        .fract();
                                    sum += if *phase < 0.5 { 1.0 } else { -1.0 };
                                }
                                (sum / 6.0) * (0.4 + 0.6 * d.tone) + (noise - d.low) * 0.25
                            }
                        };
                        let gain = 1.0 + 8.0 * d.drive;
                        value = (raw * gain).tanh() / gain.tanh() * env * 5.0 * d.level * d.accent;
                        d.age = d.age.saturating_add(1);
                    }
                }
                if let Some(out) = outputs.first_mut() {
                    *out = PortFrame::mono(value as f32);
                }
            }
            Self::Filter(f) => {
                let signal = input(inputs, 0);
                let cv = input(inputs, 1);
                let count = usize::from(signal.channels).clamp(1, MAX_CHANNELS);
                for lane in count.min(f.active)..count.max(f.active) {
                    f.states[lane] = [0.0; 2];
                    f.drive_lanes[lane] = DriveLane::new();
                }
                f.active = count;
                for out in outputs.iter_mut().take(3) {
                    out.channels = count as u8;
                }
                let gain = 1.0 + 8.0 * f.drive;
                let t = (f.drive/0.1).clamp(0.0,1.0);
                let blend = t*t*(3.0-2.0*t);
                for lane in 0..count {
                    let raw = finite(signal.voice(lane, 0.0)).clamp(-1e6, 1e6);
                    let (x, delayed_cv) = f.drive_lanes[lane].process(raw, finite(cv.voice(lane, 0.0)), gain, blend);
                    let cutoff = (f.cutoff
                        * 2.0_f64.powf((delayed_cv * f.cv).clamp(-32.0, 32.0)))
                    .clamp(1.0, f.rate * 0.45);
                    let g = (PI * cutoff / f.rate).tan();
                    let k = 2.0 - 1.95 * f.resonance;
                    let a = 1.0 / (1.0 + g * (g + k));
                    let [s1, s2] = f.states[lane];
                    let band = a * (s1 + g * (x - s2));
                    let low = s2 + g * band;
                    f.states[lane] = [
                        (2.0 * band - s1).clamp(-1e6, 1e6),
                        (2.0 * low - s2).clamp(-1e6, 1e6),
                    ];
                    for (out, value) in outputs.iter_mut().zip([low, band, x - k * band - low]) {
                        out.values[lane] = value.clamp(-1e6, 1e6) as f32;
                    }
                }
            }
        }
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        match (self, old) {
            (Self::Drum(a), Self::Drum(b)) if a.rate == b.rate && a.mode == b.mode => {
                a.high = b.high;
                a.age = b.age;
                a.active = b.active;
                a.phase = b.phase;
                a.metal = b.metal;
                a.noise = b.noise;
                a.low = b.low;
                a.accent = b.accent;
            }
            (Self::Filter(a), Self::Filter(b)) if a.rate == b.rate => {
                a.states = b.states;
                a.active = b.active;
                for (a,b) in a.drive_lanes.iter_mut().zip(&b.drive_lanes) { a.copy_state_from(b); }
            }
            _ => {}
        }
    }
    pub(crate) fn reset_state(&mut self) {
        match self {
            Self::Drum(d) => {
                d.high = false;
                d.age = 0;
                d.active = false;
                d.phase = 0.0;
                d.metal = [0.0; 6];
                d.noise = 0x92d68ca2;
                d.low = 0.0;
                d.accent = 1.0;
            }
            Self::Filter(f) => {
                f.states = [[0.0; 2]; MAX_CHANNELS];
                for lane in f.drive_lanes.iter_mut() { *lane=DriveLane::new(); }
                f.active = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(kind: &str, params: &[(&str, f32)]) -> Node {
        Node {
            score: None,
            id: "test".into(),
            kind: kind.into(),
            bypassed: false,
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }
    fn tick(tool: &mut VoiceTools, signal: f32) -> [PortFrame; 3] {
        let mut out = [PortFrame::default(); 3];
        tool.process(&[PortFrame::mono(signal)], &mut out);
        out
    }
    #[test]
    fn drum_modes_are_deterministic_decay_and_do_not_retrigger_held_gate() {
        for mode in 0..=2 {
            let n = node(
                "drum_voice",
                &[("mode", mode as f32), ("decay", 0.02), ("pitch", 300.0)],
            );
            let mut a = VoiceTools::build(&n, 48000).unwrap();
            let mut b = VoiceTools::build(&n, 48000).unwrap();
            let mut early = 0.0_f64;
            let mut late = 0.0_f64;
            for i in 0..10000 {
                let x = tick(&mut a, 10.0);
                assert_eq!(x, tick(&mut b, 10.0));
                assert!(x[0].values[0].is_finite());
                if i < 500 {
                    early += f64::from(x[0].values[0]).powi(2);
                }
                if i > 9500 {
                    late += f64::from(x[0].values[0]).powi(2);
                }
            }
            assert!(early > 0.01);
            assert!(late < early * 1e-8);
            tick(&mut a, 0.0);
            let mut retrigger_energy = 0.0;
            for _ in 0..100 {
                retrigger_energy += f64::from(tick(&mut a, 10.0)[0].values[0]).powi(2);
            }
            assert!(retrigger_energy > 0.01);
            a.reset_state();
            assert_eq!(tick(&mut a, 0.0)[0], PortFrame::mono(0.0));
        }
    }
    #[test]
    fn accent_zero_silences_and_matching_adoption_preserves_continuation() {
        let n = node("drum_voice", &[("mode", 1.0)]);
        let mut a = VoiceTools::build(&n, 48000).unwrap();
        let mut out = [PortFrame::default()];
        a.process(&[PortFrame::mono(10.0), PortFrame::mono(0.0)], &mut out);
        assert_eq!(out[0].values[0], 0.0);
        a.reset_state();
        for i in 0..100 {
            tick(&mut a, if i == 0 { 10.0 } else { 0.0 });
        }
        let mut b = VoiceTools::build(&n, 48000).unwrap();
        b.adopt(&a);
        for _ in 0..100 {
            assert_eq!(tick(&mut a, 0.0), tick(&mut b, 0.0));
        }
    }
    fn response(freq: f64) -> [f64; 3] {
        let mut f = VoiceTools::build(
            &node("svf_filter", &[("cutoff", 1000.0), ("resonance", 0.0)]),
            48000,
        )
        .unwrap();
        let mut energy = [0.0; 3];
        for i in 0..12000 {
            let out = tick(&mut f, (TAU * freq * i as f64 / 48000.0).sin() as f32);
            if i > 6000 {
                for j in 0..3 {
                    energy[j] += f64::from(out[j].values[0]).powi(2);
                }
            }
        }
        energy
    }
    #[test]
    fn svf_has_low_high_separation_and_center_band_response() {
        let low = response(100.0);
        let high = response(10000.0);
        let center = response(1000.0);
        assert!(low[0] > low[2] * 100.0);
        assert!(high[2] > high[0] * 100.0);
        assert!(center[1] > low[1] * 10.0);
        assert!(center[1] > high[1] * 10.0);
    }
    #[test]
    fn svf_polyphony_stability_nonfinite_safety_and_adoption() {
        let n = node(
            "svf_filter",
            &[
                ("cutoff", 20000.0),
                ("resonance", 1.0),
                ("cutoff_cv_amount", 4.0),
            ],
        );
        let mut a = VoiceTools::build(&n, 48000).unwrap();
        let mut signal = PortFrame {
            channels: 16,
            ..Default::default()
        };
        signal.values.fill(1.0);
        signal.values[7] = f32::NAN;
        let cv = PortFrame::mono(100.0);
        let mut out = [PortFrame::default(); 3];
        for _ in 0..10000 {
            a.process(&[signal, cv], &mut out);
            for port in out {
                assert_eq!(port.channels, 16);
                assert!(port.values.iter().all(|v| v.is_finite() && v.abs() <= 1e6));
            }
        }
        let mut b = VoiceTools::build(&n, 48000).unwrap();
        b.adopt(&a);
        let mut other = out;
        for _ in 0..30 {
            a.process(&[signal, cv], &mut out);
            b.process(&[signal, cv], &mut other);
            assert_eq!(out, other);
        }
        a.reset_state();
        assert!(tick(&mut a, 0.0)
            .iter()
            .all(|p| p.values.iter().all(|v| *v == 0.0)));
    }
    #[test]
    fn invalid_mode_and_zero_rate_are_rejected() {
        assert!(VoiceTools::build(&node("drum_voice", &[("mode", 0.5)]), 48000).is_err());
        assert!(VoiceTools::build(&node("svf_filter", &[]), 0).is_err());
    }
}
