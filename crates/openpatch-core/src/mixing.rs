//! Original voltage mixer and complementary crossfade. Reference signal roles
//! and meter packing are explicit; f64 safety arithmetic is not bit-exact SIMD.
use crate::{parameter, ModuleLightState, Node, PortFrame, SIGNAL_LIMIT};
use serde_json::{json, Value};

pub(crate) fn input_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "vca_mix" => Some(&[
            "in1", "in2", "in3", "in4", "cv1", "cv2", "cv3", "cv4", "mix_cv",
        ]),
        "fade" => Some(&["a", "b", "cv"]),
        _ => None,
    }
}
pub(crate) fn output_names(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "vca_mix" => Some(&["out1", "out2", "out3", "out4", "mix"]),
        "fade" => Some(&["out", "complement"]),
        _ => None,
    }
}
pub(crate) fn catalog() -> [Value; 2] {
    let mut mixer = json!({"kind":"vca_mix","title":"VCA MIX","model":"functional","inputs":input_names("vca_mix").unwrap(),"params":{
        "mix_level":{"default":1.0,"min":0.0,"max":2.0,"unit":"x","display":"20*log10(value) dB; zero is -infinity"},
        "channel_exp":{"default":0.0,"min":0.0,"max":1.0,"step":1.0},"mix_exp":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}},
        "description":"Four squared-fader VCAs, individual post-VCA outputs, and a linear master VCA. Strip outputs always contribute to the mix. CV clamps only below zero, not above 10 V; exponential response is fourth-power. All outputs use maximum signal width, minimum one; CV never creates voices."});
    for i in 1..=4 {
        mixer["params"][format!("level{i}")] = json!({"default":1.0,"min":0.0,"max":std::f32::consts::SQRT_2,"unit":"x","display":"40*log10(value) dB; zero is -infinity"});
    }
    [
        mixer,
        json!({"kind":"fade","title":"FADE","model":"functional","inputs":input_names("fade").unwrap(),"params":{
        "position":{"default":0.5,"min":0.0,"max":1.0},"cv_amount":{"default":0.0,"min":-1.0,"max":1.0},"law":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}},
        "description":"CV-controlled complementary crossfade: x=clamp(position+CV/10*cv_amount,0,1). Linear weights (1-x,x), or square roots for -3 dB law. Main and complementary outputs swap the same weights. Signal counts determine voices, minimum one; mono signals/CV broadcast, short poly zero-fills."}),
    ]
}
pub(crate) fn annotate(module: &mut Value) {
    let kind = module["kind"].as_str().unwrap().to_owned();
    if input_names(&kind).is_none() {
        return;
    }
    module["polyphony"]["version_2"] = json!(if kind == "vca_mix" {
        "all five outputs use max(in1,in2,in3,in4,1); mono signals and CV broadcast; short-poly lanes zero; CV width never creates voices"
    } else {
        "both outputs use max(a,b,1); mono signals/CV broadcast; missing short-poly lanes zero; CV width never creates voices"
    });
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["retired_voices"]=json!("no per-lane histories; VCA mix has four shared strip meter envelopes independent of voice count");
    module["runtime_actions"] = json!([]);
    module["arithmetic_policy"]=json!("nonfinite input voltages become zero; f64 signal arithmetic; final outputs clamp ±1,000,000 V only after full strip/master or crossfade calculation; no source f32 bit-exactness or unbounded-output claim");
    module["functional_reference"]=json!(format!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/{}.cpp",if kind=="vca_mix"{"VCMixer"}else{"Fade"}));
    if kind == "vca_mix" {
        for meta in &mut module["input_metadata"].as_array_mut().unwrap()[4..] {
            meta["normalization"] = json!(10.0);
        }
        module["gain_policy"] = json!({"strip":"signal * level^2 * CVgain","master":"sum(unclamped strips) * mix_level * CVgain","cv":"unpatched/zero-channel unity; otherwise max(0,CV/10), fourth power if corresponding exp flag is enabled; no upper clamp","level_max":"f32 sqrt(2), approximately 2x amplitude after square"});
        module["light_names"] = json!(["out1", "out2", "out3", "out4"]);
        module["light_policy"] = json!({"mode":"strip VU meters","count":4,"storage":"brightness[i][0] is 0..1 meter intensity; [i][1] always zero; manual false","input":"signed unclamped post-strip sum /5, including source SIMD-rounded groups of four; cancellation occurs before absolute value; padded mono/CV lanes are intentionally included","envelope":"per-sample absolute peak; attack immediate, release v += (abs(input)-v)*30/sample_rate","brightness":"clamp((20*log10(v)+24)/24,0,1), silence zero; refresh at every 512th processed sample and hold between updates","dependency":"https://raw.githubusercontent.com/VCVRack/Rack/061ccf63c1758599396ac1bb10d47345d9d34076/include/dsp/vumeter.hpp"});
        module["state_adoption"]=json!("same ID/kind/rate preserves all four envelope values, displayed brightness and divider phase with new parameters/wiring; bypass freezes them; fresh host identity clears meters; changing/resetting parameter values alone does not clear adopted meter histories");
    } else {
        module["law_policy"] = json!({"linear":"out=a*(1-x)+b*x; complement=a*x+b*(1-x)","sqrt":"replace weights x and 1-x by their square roots; no sine/cosine law","cv_normal":0.0});
        module["light_names"] = json!([]);
    }
}
fn finite(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value)
    } else {
        0.0
    }
}
fn voltage(value: f64) -> f32 {
    value.clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT)) as f32
}
fn cv_gain(frame: &PortFrame, lane: usize, exponential: bool) -> f64 {
    let cv = (finite(frame.voice(lane, 10.0)) / 10.0).max(0.0);
    if exponential {
        let square = cv * cv;
        square * square
    } else {
        cv
    }
}
#[derive(Clone, Debug)]
pub(crate) struct VcaMix {
    levels: [f64; 4],
    mix_level: f64,
    channel_exp: bool,
    mix_exp: bool,
    rate: f64,
    meters: [f64; 4],
    brightness: [f32; 4],
    divider: u16,
}
impl VcaMix {
    fn strip(&self, inputs: &[PortFrame; 9], strip: usize, lane: usize) -> f64 {
        finite(inputs[strip].voice(lane, 0.0))
            * self.levels[strip]
            * cv_gain(&inputs[4 + strip], lane, self.channel_exp)
    }
    fn process(&mut self, inputs: &[PortFrame; 9], outputs: &mut [PortFrame]) {
        let count = inputs[..4]
            .iter()
            .map(|f| f.channels.min(16))
            .max()
            .unwrap_or(0)
            .max(1);
        for output in outputs.iter_mut() {
            *output = PortFrame {
                channels: count,
                ..PortFrame::default()
            };
        }
        let meter_lanes = usize::from(count).div_ceil(4) * 4;
        let mut sums = [0.0; 4];
        for lane in 0..meter_lanes {
            let mut mixed = 0.0;
            for (strip, sum) in sums.iter_mut().enumerate() {
                let value = self.strip(inputs, strip, lane);
                *sum += value;
                mixed += value;
                if lane < usize::from(count) {
                    outputs[strip].values[lane] = voltage(value);
                }
            }
            if lane < usize::from(count) {
                outputs[4].values[lane] =
                    voltage(mixed * self.mix_level * cv_gain(&inputs[8], lane, self.mix_exp));
            }
        }
        for (meter, sum) in self.meters.iter_mut().zip(sums) {
            let target = (sum / 5.0).abs();
            if target >= *meter {
                *meter = target;
            } else {
                *meter += (target - *meter) * 30.0 / self.rate;
            }
        }
        self.divider += 1;
        if self.divider == 512 {
            self.divider = 0;
            for (brightness, meter) in self.brightness.iter_mut().zip(self.meters) {
                *brightness = if meter <= 0.0 {
                    0.0
                } else {
                    ((20.0 * meter.log10() + 24.0) / 24.0).clamp(0.0, 1.0) as f32
                };
            }
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) enum Mixing {
    Vca(Box<VcaMix>),
    Fade {
        position: f64,
        amount: f64,
        sqrt: bool,
    },
}
impl Mixing {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        let allowed: &[&str] = if node.kind == "vca_mix" {
            &[
                "level1",
                "level2",
                "level3",
                "level4",
                "mix_level",
                "channel_exp",
                "mix_exp",
            ]
        } else {
            &["position", "cv_amount", "law"]
        };
        for key in node.params.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let p = |name: &str, default, min, max| parameter(node, name, default, min, max);
        let flag = |name: &str| -> Result<bool, String> {
            let value = p(name, 0.0, 0.0, 1.0)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': {name} must be a boolean integer",
                    node.id
                ));
            }
            Ok(value != 0.0)
        };
        if node.kind == "fade" {
            return Ok(Self::Fade {
                position: f64::from(p("position", 0.5, 0.0, 1.0)?),
                amount: f64::from(p("cv_amount", 0.0, -1.0, 1.0)?),
                sqrt: flag("law")?,
            });
        }
        let mut levels = [0.0; 4];
        for (i, level) in levels.iter_mut().enumerate() {
            let value = f64::from(p(
                &format!("level{}", i + 1),
                1.0,
                0.0,
                std::f32::consts::SQRT_2,
            )?);
            *level = value * value;
        }
        Ok(Self::Vca(Box::new(VcaMix {
            levels,
            mix_level: f64::from(p("mix_level", 1.0, 0.0, 2.0)?),
            channel_exp: flag("channel_exp")?,
            mix_exp: flag("mix_exp")?,
            rate: f64::from(rate),
            meters: [0.0; 4],
            brightness: [0.0; 4],
            divider: 0,
        })))
    }
    pub(crate) fn lights(&self) -> Option<ModuleLightState> {
        match self {
            Self::Vca(v) => Some(ModuleLightState {
                count: 4,
                brightness: std::array::from_fn(|i| {
                    [v.brightness.get(i).copied().unwrap_or(0.0), 0.0]
                }),
                manual: false,
            }),
            _ => None,
        }
    }
    pub(crate) fn reset_state(&mut self) {
        if let Self::Vca(v) = self {
            v.meters = [0.0; 4];
            v.brightness = [0.0; 4];
            v.divider = 0;
        }
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if let (Self::Vca(v), Self::Vca(previous)) = (self, old) {
            v.meters = previous.meters;
            v.brightness = previous.brightness;
            v.divider = previous.divider;
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 9], outputs: &mut [PortFrame]) {
        match self {
            Self::Vca(v) => v.process(inputs, outputs),
            Self::Fade {
                position,
                amount,
                sqrt,
            } => {
                let count = inputs[0].channels.max(inputs[1].channels).clamp(1, 16);
                for output in outputs.iter_mut() {
                    *output = PortFrame {
                        channels: count,
                        ..PortFrame::default()
                    };
                }
                for lane in 0..usize::from(count) {
                    let x = (*position + finite(inputs[2].voice(lane, 0.0)) / 10.0 * *amount)
                        .clamp(0.0, 1.0);
                    let (left, right) = if *sqrt {
                        ((1.0 - x).sqrt(), x.sqrt())
                    } else {
                        (1.0 - x, x)
                    };
                    let a = finite(inputs[0].voice(lane, 0.0));
                    let b = finite(inputs[1].voice(lane, 0.0));
                    outputs[0].values[lane] = voltage(a * left + b * right);
                    outputs[1].values[lane] = voltage(a * right + b * left);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(kind: &str, params: &[(&str, f32)]) -> Mixing {
        Mixing::build(
            &Node {
                score: None,
                id: "s".into(),
                kind: kind.into(),
                bypassed: false,
                params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
            },
            8000,
        )
        .unwrap()
    }
    fn frame(v: &[f32]) -> PortFrame {
        let mut f = PortFrame {
            channels: v.len() as u8,
            ..PortFrame::default()
        };
        f.values[..v.len()].copy_from_slice(v);
        f
    }
    fn sample(m: &mut Mixing, inputs: &[PortFrame; 9]) -> [PortFrame; 5] {
        let mut out = [PortFrame::default(); 5];
        let count = if matches!(m, Mixing::Vca(_)) { 5 } else { 2 };
        m.process(inputs, &mut out[..count]);
        out
    }
    #[test]
    fn vca_strip_square_master_linear_and_exponential_cv_have_no_upper_clamp() {
        let mut inputs = [PortFrame::default(); 9];
        inputs[0] = PortFrame::mono(4.0);
        let mut m = model("vca_mix", &[("level1", 0.5), ("mix_level", 2.0)]);
        let out = sample(&mut m, &inputs);
        assert_eq!(out[0].values[0], 1.0);
        assert_eq!(out[4].values[0], 2.0);
        inputs[4] = PortFrame::mono(5.0);
        inputs[8] = PortFrame::mono(5.0);
        let mut m = model("vca_mix", &[("channel_exp", 1.0), ("mix_exp", 1.0)]);
        let out = sample(&mut m, &inputs);
        assert_eq!(out[0].values[0], 0.25);
        assert_eq!(out[4].values[0], 0.015625);
        inputs[4] = PortFrame::mono(20.0);
        inputs[8] = PortFrame::mono(20.0);
        let out = sample(&mut m, &inputs);
        assert_eq!(out[0].values[0], 64.0);
        assert_eq!(out[4].values[0], 1024.0);
        inputs[4] = PortFrame::mono(-10.0);
        assert_eq!(sample(&mut m, &inputs)[0].values[0], 0.0);
        inputs[4] = PortFrame::default();
        inputs[8] = PortFrame::default();
        assert_eq!(sample(&mut m, &inputs)[4].values[0], 4.0);
        let mut maximum = model("vca_mix", &[("level1", std::f32::consts::SQRT_2)]);
        assert!((sample(&mut maximum, &inputs)[0].values[0] - 8.0).abs() < 1e-6);
    }
    #[test]
    fn rounded_meter_lanes_include_mono_cv_padding_and_signed_cancellation() {
        let mut inputs = [PortFrame::default(); 9];
        inputs[0] = PortFrame::mono(0.25);
        let mut m = model("vca_mix", &[]);
        sample(&mut m, &inputs);
        let Mixing::Vca(v) = &m else { unreachable!() };
        assert_eq!(v.meters[0], 0.2);
        inputs[1] = frame(&[0.0; 5]);
        let mut m = model("vca_mix", &[]);
        sample(&mut m, &inputs);
        let Mixing::Vca(v) = &m else { unreachable!() };
        assert_eq!(v.meters[0], 0.4);
        inputs[1] = PortFrame::default();
        inputs[0] = PortFrame::mono(1.0);
        inputs[4] = frame(&[10.0, 20.0]);
        let mut m = model("vca_mix", &[]);
        let out = sample(&mut m, &inputs);
        assert_eq!(out[0], PortFrame::mono(1.0));
        let Mixing::Vca(v) = &m else { unreachable!() };
        assert_eq!(v.meters[0], 0.6);
        inputs[0] = frame(&[1.0, -1.0]);
        inputs[4] = PortFrame::default();
        let mut m = model("vca_mix", &[]);
        for _ in 0..512 {
            sample(&mut m, &inputs);
        }
        assert_eq!(
            m.lights().unwrap().brightness,
            [[0.0; 2]; crate::MAX_MODULE_LIGHTS]
        );
    }
    #[test]
    fn peak_envelope_brightness_divider_and_release_recurrence_are_exact_policies() {
        let mut inputs = [PortFrame::default(); 9];
        inputs[0] = PortFrame::mono(0.25);
        let mut m = model("vca_mix", &[]);
        for _ in 0..511 {
            sample(&mut m, &inputs);
        }
        assert_eq!(m.lights().unwrap().brightness[0][0], 0.0);
        sample(&mut m, &inputs);
        let expected = ((20_f64 * 0.2_f64.log10() + 24.0) / 24.0) as f32;
        assert!((m.lights().unwrap().brightness[0][0] - expected).abs() < 1e-7);
        inputs[0] = PortFrame::mono(0.0);
        sample(&mut m, &inputs);
        assert_eq!(m.lights().unwrap().brightness[0][0], expected);
        let Mixing::Vca(v) = &m else { unreachable!() };
        assert!((v.meters[0] - 0.2 * (1.0 - 30.0 / 8000.0)).abs() < 1e-15);
        for _ in 0..511 {
            sample(&mut m, &inputs);
        }
        let envelope = 0.2 * (1.0_f64 - 30.0 / 8000.0).powi(512);
        let expected = ((20.0 * envelope.log10() + 24.0) / 24.0).clamp(0.0, 1.0) as f32;
        assert!((m.lights().unwrap().brightness[0][0] - expected).abs() < 1e-7);
        m.reset_state();
        assert_eq!(
            m.lights().unwrap().brightness,
            [[0.0; 2]; crate::MAX_MODULE_LIGHTS]
        );
    }
    #[test]
    fn fade_complement_cv_clamp_signed_amount_and_square_root_law_are_distinct() {
        let mut inputs = [PortFrame::default(); 9];
        inputs[0] = PortFrame::mono(8.0);
        inputs[1] = PortFrame::mono(2.0);
        for (position, expected) in [
            (0.0, [8.0, 2.0]),
            (0.25, [6.5, 3.5]),
            (0.5, [5.0, 5.0]),
            (1.0, [2.0, 8.0]),
        ] {
            let mut f = model("fade", &[("position", position)]);
            let out = sample(&mut f, &inputs);
            assert_eq!([out[0].values[0], out[1].values[0]], expected);
        }
        let mut f = model("fade", &[("position", 0.25), ("law", 1.0)]);
        let out = sample(&mut f, &inputs);
        assert!((out[0].values[0] - (8.0 * 0.75_f32.sqrt() + 1.0)).abs() < 1e-6);
        assert!((out[1].values[0] - (4.0 + 2.0 * 0.75_f32.sqrt())).abs() < 1e-6);
        inputs[2] = PortFrame::mono(20.0);
        let mut f = model("fade", &[("cv_amount", 1.0)]);
        assert_eq!(sample(&mut f, &inputs)[0].values[0], 2.0);
        let mut f = model("fade", &[("cv_amount", -1.0)]);
        assert_eq!(sample(&mut f, &inputs)[0].values[0], 8.0);
    }
    #[test]
    fn extreme_finite_cv_and_cancellation_keep_outputs_and_meter_state_finite() {
        let mut asymmetric = [PortFrame::default(); 9];
        asymmetric[0] = PortFrame::mono(2_000_000.0);
        asymmetric[1] = PortFrame::mono(-1_500_000.0);
        let mut unity = model("vca_mix", &[]);
        let out = sample(&mut unity, &asymmetric);
        assert_eq!(out[0].values[0], SIGNAL_LIMIT);
        assert_eq!(out[1].values[0], -SIGNAL_LIMIT);
        assert_eq!(out[4].values[0], 500_000.0);
        let mut attenuated = model("vca_mix", &[("mix_level", 0.1)]);
        assert_eq!(sample(&mut attenuated, &asymmetric)[4].values[0], 50_000.0);
        let mut inputs = [PortFrame::default(); 9];
        inputs[0] = PortFrame::mono(f32::MAX);
        inputs[1] = PortFrame::mono(-f32::MAX);
        inputs[4] = PortFrame::mono(f32::MAX);
        inputs[5] = inputs[4];
        inputs[8] = inputs[4];
        let mut m = model("vca_mix", &[("channel_exp", 1.0), ("mix_exp", 1.0)]);
        let out = sample(&mut m, &inputs);
        assert_eq!(out[0].values[0], SIGNAL_LIMIT);
        assert_eq!(out[1].values[0], -SIGNAL_LIMIT);
        assert_eq!(out[4].values[0], 0.0);
        inputs[1] = PortFrame::default();
        assert_eq!(sample(&mut m, &inputs)[4].values[0], SIGNAL_LIMIT);
        let Mixing::Vca(v) = &m else { unreachable!() };
        assert!(v.meters.iter().all(|v| v.is_finite()));
        inputs[0] = PortFrame::mono(f32::NAN);
        inputs[4] = PortFrame::mono(f32::INFINITY);
        assert_eq!(sample(&mut m, &inputs)[4].values[0], 0.0);
        inputs[0] = PortFrame::mono(f32::MAX);
        inputs[1] = PortFrame::mono(-f32::MAX);
        let mut f = model("fade", &[]);
        assert_eq!(sample(&mut f, &inputs)[0].values[0], 0.0);
        inputs[1] = inputs[0];
        assert_eq!(sample(&mut f, &inputs)[0].values[0], SIGNAL_LIMIT);
    }
}
