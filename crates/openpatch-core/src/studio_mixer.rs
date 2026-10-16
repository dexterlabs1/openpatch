//! Bounded stereo performance console: 1–16 strips, two stereo aux sends and
//! returns. Disabled strips retain settings/wiring and emit no signal.
use crate::{parameter, ModuleLightState, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};
pub(crate) const INPUTS: [&str; 20] = [
    "in1",
    "in2",
    "in3",
    "in4",
    "in5",
    "in6",
    "in7",
    "in8",
    "in9",
    "in10",
    "in11",
    "in12",
    "in13",
    "in14",
    "in15",
    "in16",
    "return_a",
    "return_b",
    "send_a_cv",
    "send_b_cv",
];
pub(crate) const OUTPUTS: [&str; 8] = [
    "left",
    "right",
    "send_a_left",
    "send_a_right",
    "send_b_left",
    "send_b_right",
    "cue_left",
    "cue_right",
];
pub(crate) fn catalog() -> Value {
    let mut c = json!({"kind":"studio_mixer","title":"STUDIO MIXER","model":"functional","inputs":INPUTS.to_vec(),"description":"Resizable 1–16-channel stereo console with trim, low/high shelves, high-pass, balance/pan, smoothed faders, mute, solo, two pre/post aux sends, stereo returns, master and pre-master cue. Mono inputs center at unity; two-lane cables are stereo; larger poly cables are folded to mono with 1/sqrt(N) gain. Shrinking retains inactive strip settings and cables. Mute/solo affect sends as well as main. Returns never feed sends; no internal feedback loop.","params":{
 "channels":{"default":8,"min":1,"max":16,"step":1},"master":{"default":0.8,"min":0,"max":2},"return_a":{"default":1,"min":0,"max":2},"return_b":{"default":1,"min":0,"max":2},"send_a_master":{"default":1,"min":0,"max":2},"send_b_master":{"default":1,"min":0,"max":2}}});
    c["tags"] = json!(["Custom", "Mixing"]);
    for i in 1..=16 {
        for (key, default, min, max, step) in [
            ("level", 1., 0., 2., false),
            ("trim", 1., 0., 2., false),
            ("pan", 0., -1., 1., false),
            ("mute", 0., 0., 1., true),
            ("solo", 0., 0., 1., true),
            ("send_a", 0., 0., 1., false),
            ("send_b", 0., 0., 1., false),
            ("pre_a", 0., 0., 1., true),
            ("pre_b", 0., 0., 1., true),
            ("low_db", 0., -12., 12., false),
            ("high_db", 0., -12., 12., false),
            ("hpf", 20., 20., 1000., false),
        ] {
            let mut spec = json!({"default":default,"min":min,"max":max});
            if step {
                spec["step"] = json!(1);
            }
            if key.ends_with("db") {
                spec["unit"] = json!("dB");
            }
            if key == "hpf" {
                spec["unit"] = json!("Hz");
            }
            c["params"][format!("{key}{i}")] = spec;
        }
    }
    c
}
fn clean(v: f32) -> f64 {
    if v.is_finite() {
        f64::from(v).clamp(-1e6, 1e6)
    } else {
        0.
    }
}
fn stereo(p: PortFrame) -> [f64; 2] {
    match p.channels {
        0 => [0.; 2],
        1 => [clean(p.values[0]); 2],
        2 => [clean(p.values[0]), clean(p.values[1])],
        _ => {
            let x = p.values[..usize::from(p.channels.min(16))]
                .iter()
                .map(|&v| clean(v))
                .sum::<f64>()
                / f64::from(p.channels).sqrt();
            [x; 2]
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct StripState {
    level: f64,
    pan: f64,
    send: [f64; 2],
    enabled: f64,
    low: [f64; 2],
    high: [f64; 2],
    dc: [f64; 2],
    meter: [f64; 2],
}
#[derive(Clone, Copy, Debug)]
struct Strip {
    level: f64,
    trim: f64,
    pan: f64,
    mute: bool,
    solo: bool,
    send: [f64; 2],
    pre: [bool; 2],
    low: f64,
    high: f64,
    hpf: f64,
}
#[derive(Debug)]
pub(crate) struct StudioMixer {
    rate: u32,
    channels: usize,
    strips: [Strip; 16],
    state: [StripState; 16],
    master: f64,
    returns: [f64; 2],
    send_master: [f64; 2],
    master_state: f64,
    return_state: [f64; 2],
    smooth: f64,
    low_pole: f64,
    high_pole: f64,
}
impl StudioMixer {
    pub(crate) fn build(n: &Node, rate: u32) -> Result<Self, String> {
        if rate == 0 {
            return Err("mixer needs a positive sample rate".into());
        }
        let schema = catalog();
        for k in n.params.keys() {
            if schema["params"].get(k).is_none() {
                return Err(format!("unknown mixer parameter {k}"));
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
        let channels = p("channels")?;
        if channels.fract() != 0. {
            return Err("mixer channel count must be an integer".into());
        }
        let mut strips = Vec::with_capacity(16);
        for i in 1..=16 {
            let q = |s: &str| p(&format!("{s}{i}"));
            for key in ["mute", "solo", "pre_a", "pre_b"] {
                if q(key)?.fract() != 0. {
                    return Err(format!("mixer {key} must be 0 or 1"));
                }
            }
            strips.push(Strip {
                level: q("level")?,
                trim: q("trim")?,
                pan: q("pan")?,
                mute: q("mute")? != 0.,
                solo: q("solo")? != 0.,
                send: [q("send_a")?, q("send_b")?],
                pre: [q("pre_a")? != 0., q("pre_b")? != 0.],
                low: 10f64.powf(q("low_db")? / 20.),
                high: 10f64.powf(q("high_db")? / 20.),
                hpf: 1.
                    - (-std::f64::consts::TAU * q("hpf")?.min(f64::from(rate) * 0.45)
                        / f64::from(rate))
                    .exp(),
            });
        }
        let strips: [Strip; 16] = strips.try_into().unwrap();
        let master = p("master")?;
        let returns = [p("return_a")?, p("return_b")?];
        Ok(Self {
            rate,
            channels: channels as usize,
            state: std::array::from_fn(|i| StripState {
                level: strips[i].level,
                pan: strips[i].pan,
                send: strips[i].send,
                enabled: if strips[i].mute { 0. } else { 1. },
                ..Default::default()
            }),
            strips,
            master,
            returns,
            send_master: [p("send_a_master")?, p("send_b_master")?],
            master_state: master,
            return_state: returns,
            smooth: 1. - (-1. / (0.005 * f64::from(rate))).exp(),
            low_pole: 1. - (-std::f64::consts::TAU * 180. / f64::from(rate)).exp(),
            high_pole: 1. - (-std::f64::consts::TAU * 4500. / f64::from(rate)).exp(),
        })
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let mut main = [0.; 2];
        let mut sends = [[0.; 2]; 2];
        let mut cue = [0.; 2];
        let solo = self.strips[..self.channels].iter().any(|s| s.solo);
        for i in 0..self.channels {
            let c = self.strips[i];
            let s = &mut self.state[i];
            let target = if c.mute || (solo && !c.solo) { 0. } else { 1. };
            s.enabled += (target - s.enabled) * self.smooth;
            s.level += (c.level - s.level) * self.smooth;
            s.pan += (c.pan - s.pan) * self.smooth;
            for j in 0..2 {
                s.send[j] += (c.send[j] - s.send[j]) * self.smooth;
            }
            let mut signal = stereo(inputs.get(i).copied().unwrap_or_default());
            let pan = [(1. - s.pan).min(1.).sqrt(), (1. + s.pan).min(1.).sqrt()];
            for ch in 0..2 {
                let x = signal[ch] * c.trim;
                s.dc[ch] += c.hpf * (x - s.dc[ch]);
                let x = x - s.dc[ch];
                s.low[ch] += self.low_pole * (x - s.low[ch]);
                s.high[ch] += self.high_pole * (x - s.high[ch]);
                signal[ch] = (x + s.low[ch] * (c.low - 1.) + (x - s.high[ch]) * (c.high - 1.))
                    * pan[ch]
                    * s.enabled;
                let post = signal[ch] * s.level;
                s.meter[ch] =
                    (post.abs() / 5.).max(s.meter[ch] * (1. - 15. / f64::from(self.rate)));
                main[ch] += post;
                if c.solo {
                    cue[ch] += post;
                }
                for (j, send) in sends.iter_mut().enumerate() {
                    send[ch] += if c.pre[j] { signal[ch] } else { post } * s.send[j];
                }
            }
        }
        for (j, send) in sends.iter_mut().enumerate() {
            self.return_state[j] += (self.returns[j] - self.return_state[j]) * self.smooth;
            let ret = stereo(inputs.get(16 + j).copied().unwrap_or_default());
            for ch in 0..2 {
                main[ch] += ret[ch] * self.return_state[j];
            }
            let cv = inputs
                .get(18 + j)
                .filter(|p| p.channels > 0)
                .map_or(1., |p| (clean(p.values[0]) / 10.).clamp(0., 1.));
            for value in send {
                *value *= self.send_master[j] * cv;
            }
        }
        if !solo {
            cue = main;
        }
        self.master_state += (self.master - self.master_state) * self.smooth;
        let values = [
            main[0] * self.master_state,
            main[1] * self.master_state,
            sends[0][0],
            sends[0][1],
            sends[1][0],
            sends[1][1],
            cue[0],
            cue[1],
        ];
        for (o, v) in outputs.iter_mut().zip(values) {
            *o = PortFrame::mono(v.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT)) as f32);
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            count: self.channels as u8,
            brightness: std::array::from_fn(|i| {
                std::array::from_fn(|ch| {
                    if i < self.channels {
                        ((20. * self.state[i].meter[ch].max(1e-10).log10() + 48.) / 48.)
                            .clamp(0., 1.) as f32
                    } else {
                        0.
                    }
                })
            }),
            ..Default::default()
        }
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate == old.rate {
            self.state = old.state;
            self.master_state = old.master_state;
            self.return_state = old.return_state;
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.state = std::array::from_fn(|i| StripState {
            level: self.strips[i].level,
            pan: self.strips[i].pan,
            send: self.strips[i].send,
            enabled: if self.strips[i].mute { 0. } else { 1. },
            ..Default::default()
        });
        self.master_state = self.master;
        self.return_state = self.returns;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mixer(params: Value) -> StudioMixer {
        let n = serde_json::from_value(json!({"id":"m","kind":"studio_mixer","params":params}))
            .unwrap();
        StudioMixer::build(&n, 48000).unwrap()
    }
    fn render(m: &mut StudioMixer, source: usize) -> [f64; 8] {
        let mut power = [0.; 8];
        let mut inputs = [PortFrame::default(); 20];
        let mut outputs = [PortFrame::default(); 8];
        for i in 0..4800 {
            inputs[source] =
                PortFrame::mono((std::f32::consts::TAU * i as f32 * 220. / 48000.).sin());
            m.process(&inputs, &mut outputs);
            if i >= 2400 {
                for j in 0..8 {
                    power[j] += f64::from(outputs[j].values[0]).powi(2);
                }
            }
        }
        power
    }
    #[test]
    fn resize_mute_solo_and_pan_route_the_selected_strip() {
        let mut m = mixer(json!({"channels":16,"pan16":1}));
        let p = render(&mut m, 15);
        assert!(p[1] > 100. && p[0] < 1e-12);
        let mut small = mixer(json!({"channels":1,"pan16":1}));
        small.adopt(&m);
        assert!(render(&mut small, 15).iter().all(|x| *x == 0.));
        let mut solo = mixer(json!({"channels":16,"solo1":1}));
        assert!(render(&mut solo, 15)[1] < 1e-7);
        let mut muted = mixer(json!({"channels":16,"mute16":1,"send_a16":1}));
        assert!(render(&mut muted, 15).iter().all(|x| *x == 0.));
    }
    #[test]
    fn pre_and_post_sends_respect_faders_and_returns_do_not_feed_sends() {
        let mut pre = mixer(json!({"channels":1,"level1":0,"send_a1":1,"pre_a1":1}));
        let p = render(&mut pre, 0);
        assert!(p[2] > 100. && p[0] == 0.);
        let mut post = mixer(json!({"channels":1,"level1":0,"send_a1":1}));
        assert_eq!(render(&mut post, 0), [0.; 8]);
        let mut ret = mixer(json!({"channels":1}));
        let p = render(&mut ret, 16);
        assert!(p[0] > 100. && p[1] > 100. && p[2..6].iter().all(|x| *x == 0.));
    }
}
