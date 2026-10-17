// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from VCV Fundamental, Andrew Belt and contributors; see LICENSE.
//! Scalar realization of the Fundamental 2.6.4 VCO numerical contract at
//! e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e. This identity deliberately retains
//! its curved waves, square latch, original event ordering and quartet lifecycle.
//! The immutable step profile is the existing offline Rack SDK 2.6.6 numerical
//! output; see scripts/wavetable_kernel/README.md for provenance and reproduction.
use crate::{
    parameter, wavetable_minblep::SYNC_BLEP, ModuleLightState, Node, PortFrame, MAX_CHANNELS,
};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 4] = ["pitch", "fm", "sync", "pwm"];
pub(crate) const OUTPUTS: [&str; 4] = ["sine", "triangle", "saw", "square"];
const C4: f32 = 261.6256;
const LENGTH: usize = 32;
const OVERSAMPLE: usize = 16;

pub(crate) fn catalog() -> Value {
    json!({"kind":"rack264_vco","title":"RACK 2.6.4 VCO","model":"functional","description":"Fundamental 2.6.4 VCO source target: curved analog-mode waves, original minimum-phase step corrections, hard/soft sync and square-only DC filtering. Qualification is source-version specific.","inputs":INPUTS,"params":{
        "frequency":{"default":0.0,"min":-54.0,"max":54.0,"unit":"semitones","description":"C4 × 2^(frequency/12) Hz before pitch/FM"},
        "pulse_width":{"default":0.5,"min":0.01,"max":0.99,"unit":"ratio"},
        "fm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Signed exponential/linear FM depth; Randomize disabled"},
        "pwm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"PWM/10 times signed depth; Randomize disabled"},
        "linear_fm":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1]},
        "soft_sync":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1]}
    }})
}
pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src/VCO.cpp");
    module["polyphony"]["primary_input"] = json!("pitch");
    module["polyphony"]["version_2"] = json!(
        "max(pitch width,1), at most16; mono inputs broadcast; missing short-poly lanes read zero"
    );
    module["polyphony"]["retired_voices"] = json!("active quartets advance all lanes, all four correction rings and square filters; only visible lanes insert corrections and hard-sync square resets; wholly inactive quartets freeze without retirement reset");
    module["consumer_policy"] = json!("disconnected outputs expose zero channels and zero host storage; all internal waves, corrections and square filters continue independently of consumers; inactive raw Rack output storage is outside voltage equivalence");
    module["frequency_policy"] = json!("source frequency control ±54 semitones (±4.5V/oct); SDK f32 fifth-order exp2, C4 and signed exponential/linear FM; frequency clamps0..sample_rate/2, phase delta0..0.35");
    module["unsupported_domain_policy"] = json!("nonfinite input CV becomes zero; unsupported exp2 arguments saturate[-127,128], and indeterminate overflow frequency stops phase; finite ordinary source-domain CV is not amplitude-clamped; these safety extensions are not SDK equivalence claims");
    module["reconstruction_policy"] = json!("immutable genuine SDK16×32 minimum-phase step profile; original wrap, PWM latch, half-cycle saw, then sync correction order; analog-mode quadratic sine and rational curved triangle/saw; only square has SDK trapezoidal20Hz highpass and0.95 gain");
    module["light_names"] = json!([
        "phase_red",
        "phase_green",
        "phase_blue",
        "linear_fm",
        "soft_sync"
    ]);
    module["light_policy"] = json!("every16 frames: mono SDK SIMD sine phase indicator with signed30/s release, or poly blue; linear/soft mode lamps; bypass freezes runtime");
    module["initialization_policy"] = json!("no source runtime-reset override: Initialize restores host controls while preserving phase, sync, square latch/filter, correction rings, lamps and divider");
    module["state_adoption"] = json!("same identity/kind/sample rate preserves bounded runtime; receiving controls win; sync routing changes preserve detector history; fresh/rate changes start fresh");
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
#[allow(clippy::excessive_precision)]
fn exp2(pitch: f32) -> f32 {
    let biased = pitch.clamp(-127.0, 128.0) + 127.0;
    let integer = biased as u32;
    let x = biased - integer as f32;
    let mut y = 0.001879100722_f32;
    for coefficient in [
        0.008991698010,
        0.055817908652,
        0.2401595990753,
        0.69315169353961,
        1.0,
    ] {
        y = coefficient + y * x;
    }
    f32::from_bits(integer << 23) * y
}
fn curve(x: f32) -> f32 {
    (3.0 + x * (-13.0 + 5.0 * x)) / (3.0 + 2.0 * x)
}
fn wave(wave: usize, phase: f32) -> f32 {
    match wave {
        0 => {
            let first = phase < 0.5;
            let x = phase - if first { 0.25 } else { 0.75 };
            (1.0 - 16.0 * (x * x)) * if first { 1.0 } else { -1.0 }
        }
        1 => {
            let mut x = phase + 0.25;
            x -= x.trunc();
            let second = x >= 0.5;
            x *= 2.0;
            x -= x.trunc();
            curve(x) * if second { 1.0 } else { -1.0 }
        }
        2 => {
            let mut x = phase + 0.5;
            x -= x.trunc();
            -curve(x)
        }
        _ => unreachable!("square is a stateful latch"),
    }
}

// Cephes range reduction/polynomial coefficients used by SDK simd::sin. Only
// the oscillator's bounded phase domain [0,1) is needed here. The evaluation
// keeps individual f32 operations, including the sign bit at the zero crossing.
#[allow(clippy::excessive_precision, clippy::approx_constant)]
fn phase_light(phase: f32) -> f32 {
    let mut x = std::f32::consts::TAU * phase;
    let quadrant = ((x * 1.27323954473516_f32) as u32 + 1) & !1;
    let y = quadrant as f32;
    x += y * -0.78515625;
    x += y * -2.4187564849853515625e-4_f32;
    x += y * -3.77489497744594108e-8_f32;
    let z = x * x;
    let result = if quadrant & 2 == 0 {
        (((-1.9515295891e-4_f32 * z + 8.3321608736e-3_f32) * z - 1.6666654611e-1_f32) * z) * x + x
    } else {
        (((2.443315711809948e-5_f32 * z - 1.388731625493765e-3_f32) * z + 4.166664568298827e-2_f32)
            * z)
            * z
            - z * 0.5
            + 1.0
    };
    f32::from_bits(result.to_bits() ^ ((quadrant & 4) << 29))
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    phase: f32,
    frequency: f32,
    width: f32,
    direction: f32,
    last_sync: f32,
    square: f32,
    filter_input: f32,
    filter_output: f32,
    tails: [[f32; LENGTH]; 4],
    cursor: u8,
    values: [f32; 4],
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            phase: 0.0,
            frequency: 0.0,
            width: 0.5,
            direction: 1.0,
            last_sync: 0.0,
            square: 1.0,
            filter_input: 0.0,
            filter_output: 0.0,
            tails: [[0.0; LENGTH]; 4],
            cursor: 0,
            values: [0.0; 4],
        }
    }
}
impl Voice {
    // p is relative to the current sample, with the SDK's strict -1 < p <=0.
    fn insert(&mut self, output: usize, p: f32, magnitude: f32) {
        if !(-1.0 < p && p <= 0.0) {
            return;
        }
        for j in 0..LENGTH {
            let position = (j as f32 - p) * OVERSAMPLE as f32;
            let integer = position as usize;
            let fraction = position - integer as f32;
            // Rounding of j-p can reach the appended endpoint even though p>-1.
            // Its interpolation weight is then zero; avoid the SDK's unused
            // one-past-end read and use the defined unit endpoint directly.
            let step = if integer >= SYNC_BLEP.len() - 1 {
                1.0
            } else {
                SYNC_BLEP[integer] + (SYNC_BLEP[integer + 1] - SYNC_BLEP[integer]) * fraction
            };
            self.tails[output][(usize::from(self.cursor) + j) % LENGTH] +=
                magnitude * (-1.0 + step);
        }
    }
    // Scalar Rack clamp uses fmin/fmax, whose NaN result differs from clamp().
    #[allow(clippy::manual_clamp)]
    fn process(
        &mut self,
        sample_time: f32,
        filter_c: f32,
        soft: bool,
        sync: Option<f32>,
        visible: bool,
    ) {
        let mut delta = (self.frequency * sample_time).clamp(0.0, 0.35);
        if soft {
            delta *= self.direction;
        } else {
            self.direction = 1.0;
        }
        self.phase += delta;
        let floor = self.phase.floor();
        self.phase -= floor;
        if floor != 0.0 {
            if visible {
                let wrap_phase = if self.direction == -1.0 { 1.0 } else { 0.0 };
                let crossing = (wrap_phase - (self.phase - delta)) / delta;
                self.insert(3, (crossing - 1.0).min(0.0).max(-1.0), 2.0 * self.direction);
            }
            self.square = self.direction;
        }
        if self.direction == self.square && ((self.direction == 1.0) ^ (self.phase < self.width)) {
            if visible {
                let crossing = (self.width - (self.phase - delta)) / delta;
                self.insert(
                    3,
                    (crossing - 1.0).min(0.0).max(-1.0),
                    -2.0 * self.direction,
                );
            }
            self.square = -self.direction;
        }
        let half_crossing = (0.5 - (self.phase - delta)) / delta;
        if visible && 0.0 < half_crossing && half_crossing <= 1.0 {
            self.insert(2, half_crossing - 1.0, -2.0 * self.direction);
        }
        if let Some(value) = sync {
            let crossing = -self.last_sync / (value - self.last_sync);
            self.last_sync = value;
            if 0.0 < crossing && crossing <= 1.0 && value >= 0.0 {
                if soft {
                    // This version reverses the NEXT frame, without splitting this frame.
                    self.direction = -self.direction;
                } else {
                    let new_phase = (1.0 - crossing) * delta;
                    if visible {
                        self.insert(3, crossing - 1.0, 1.0 - self.square);
                        self.square = 1.0;
                        for output in [2, 1, 0] {
                            self.insert(
                                output,
                                crossing - 1.0,
                                wave(output, new_phase) - wave(output, self.phase),
                            );
                        }
                    }
                    self.phase = new_phase;
                }
            }
        }
        let cursor = usize::from(self.cursor);
        for output in 0..4 {
            let raw = if output == 3 {
                self.square
            } else {
                wave(output, self.phase)
            };
            self.values[output] = raw + self.tails[output][cursor];
            self.tails[output][cursor] = 0.0;
        }
        self.cursor = ((cursor + 1) % LENGTH) as u8;
        let square = self.values[3];
        self.filter_output =
            (square + self.filter_input - self.filter_output * (1.0 - filter_c)) / (1.0 + filter_c);
        self.filter_input = square;
        self.values[3] = (self.filter_input - self.filter_output) * 0.95;
    }
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Rack264VcoSnapshot {
    pub phases: [f32; MAX_CHANNELS],
    pub frequencies: [f32; MAX_CHANNELS],
    pub pulse_widths: [f32; MAX_CHANNELS],
    pub sync_directions: [f32; MAX_CHANNELS],
    pub last_sync: [f32; MAX_CHANNELS],
    pub square_states: [f32; MAX_CHANNELS],
    pub filter_input: [f32; MAX_CHANNELS],
    pub filter_output: [f32; MAX_CHANNELS],
    pub correction_tails: [[[f32; LENGTH]; MAX_CHANNELS]; 4],
    pub buffer_indices: [u8; 4],
    pub wave_values: [[f32; MAX_CHANNELS]; 4],
    pub light_divider: u8,
    pub lights: [f32; 5],
}
#[derive(Debug)]
pub(crate) struct Rack264Vco {
    sample_time: f32,
    frequency_limit: f32,
    filter_c: f32,
    frequency: f32,
    fm_amount: f32,
    pulse_width: f32,
    pwm_amount: f32,
    linear: bool,
    soft: bool,
    voices: [Voice; MAX_CHANNELS],
    divider: u8,
    lamps: [f32; 5],
}
impl Rack264Vco {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("rack264_vco sample rate must be8000..=192000 Hz".into());
        }
        for name in node.params.keys() {
            if ![
                "frequency",
                "pulse_width",
                "fm_amount",
                "pwm_amount",
                "linear_fm",
                "soft_sync",
            ]
            .contains(&name.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let flag = |name| -> Result<bool, String> {
            let value = parameter(node, name, 0.0, 0.0, 1.0)?;
            if value.fract() != 0.0 {
                return Err(format!("node '{}': '{name}' must be an integer", node.id));
            }
            Ok(value > 0.0)
        };
        let sample_time = 1.0 / sample_rate as f32;
        Ok(Self {
            sample_time,
            frequency_limit: sample_rate as f32 / 2.0,
            filter_c: 2.0 / (std::f32::consts::TAU * (20.0 * sample_time)),
            frequency: parameter(node, "frequency", 0.0, -54.0, 54.0)? / 12.0,
            fm_amount: parameter(node, "fm_amount", 0.0, -1.0, 1.0)?,
            pulse_width: parameter(node, "pulse_width", 0.5, 0.01, 0.99)?,
            pwm_amount: parameter(node, "pwm_amount", 0.0, -1.0, 1.0)?,
            linear: flag("linear_fm")?,
            soft: flag("soft_sync")?,
            voices: [Voice::default(); MAX_CHANNELS],
            divider: 0,
            lamps: [0.0; 5],
        })
    }
    pub(crate) fn initialize_state(&mut self) { /* Source has no runtime reset override. */
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_time == old.sample_time {
            self.voices = old.voices;
            self.divider = old.divider;
            self.lamps = old.lamps;
        }
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 5,
            ..Default::default()
        };
        for (i, value) in self.lamps.iter().enumerate() {
            lights.brightness[i][0] = *value;
        }
        lights
    }
    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 4],
        mask: u16,
        outputs: &mut [PortFrame],
    ) {
        let channels = inputs[0].channels.clamp(1, 16);
        for (index, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame {
                channels: if mask & (1 << index) != 0 {
                    channels
                } else {
                    0
                },
                ..Default::default()
            };
        }
        for lane in 0..usize::from(channels).div_ceil(4) * 4 {
            let pitch = self.frequency + finite(inputs[0].voice(lane, 0.0));
            let fm = finite(inputs[1].voice(lane, 0.0));
            let frequency = if self.linear {
                C4 * exp2(pitch) + C4 * fm * self.fm_amount
            } else {
                C4 * exp2(pitch + fm * self.fm_amount)
            };
            let frequency = if frequency.is_nan() {
                0.0
            } else {
                frequency.clamp(0.0, self.frequency_limit)
            };
            let voice = &mut self.voices[lane];
            voice.frequency = frequency;
            voice.width = (self.pulse_width
                + finite(inputs[3].voice(lane, 0.0)) / 10.0 * self.pwm_amount)
                .clamp(0.01, 0.99);
            let sync = (inputs[2].channels != 0).then(|| finite(inputs[2].voice(lane, 0.0)));
            voice.process(
                self.sample_time,
                self.filter_c,
                self.soft,
                sync,
                lane < usize::from(channels),
            );
            if lane < usize::from(channels) {
                for (index, output) in outputs.iter_mut().enumerate() {
                    if mask & (1 << index) != 0 {
                        output.values[lane] = 5.0 * voice.values[index];
                    }
                }
            }
        }
        self.divider += 1;
        if self.divider == 16 {
            self.divider = 0;
            if channels == 1 {
                let phase = phase_light(self.voices[0].phase);
                for (index, target) in [-phase, phase].into_iter().enumerate() {
                    if target < self.lamps[index] {
                        self.lamps[index] +=
                            (target - self.lamps[index]) * 30.0 * (self.sample_time * 16.0);
                    } else {
                        self.lamps[index] = target;
                    }
                }
                self.lamps[2] = 0.0;
            } else {
                self.lamps[0] = 0.0;
                self.lamps[1] = 0.0;
                self.lamps[2] = 1.0;
            }
            self.lamps[3] = u8::from(self.linear) as f32;
            self.lamps[4] = u8::from(self.soft) as f32;
        }
    }
    fn snapshot(&self) -> Rack264VcoSnapshot {
        Rack264VcoSnapshot {
            phases: std::array::from_fn(|i| self.voices[i].phase),
            frequencies: std::array::from_fn(|i| self.voices[i].frequency),
            pulse_widths: std::array::from_fn(|i| self.voices[i].width),
            sync_directions: std::array::from_fn(|i| self.voices[i].direction),
            last_sync: std::array::from_fn(|i| self.voices[i].last_sync),
            square_states: std::array::from_fn(|i| self.voices[i].square),
            filter_input: std::array::from_fn(|i| self.voices[i].filter_input),
            filter_output: std::array::from_fn(|i| self.voices[i].filter_output),
            correction_tails: std::array::from_fn(|w| {
                std::array::from_fn(|i| {
                    std::array::from_fn(|s| {
                        self.voices[i].tails[w][(usize::from(self.voices[i].cursor) + s) % LENGTH]
                    })
                })
            }),
            buffer_indices: std::array::from_fn(|g| self.voices[g * 4].cursor),
            wave_values: std::array::from_fn(|w| std::array::from_fn(|i| self.voices[i].values[w])),
            light_divider: self.divider,
            lights: self.lamps,
        }
    }
}

/// Diagnostics of the production processor, with logical future correction tails.
#[doc(hidden)]
#[derive(Debug)]
pub struct Rack264VcoProbe(Rack264Vco);
impl Rack264VcoProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        Rack264Vco::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 4], mask: u16) -> [PortFrame; 4] {
        let mut outputs = [PortFrame::default(); 4];
        self.0.process(inputs, mask, &mut outputs);
        outputs
    }
    pub fn snapshot(&self) -> Rack264VcoSnapshot {
        self.0.snapshot()
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn adopt(&mut self, old: &Self) {
        self.0.adopt(&old.0);
    }
    pub fn kernel(&self) -> [f32; 513] {
        SYNC_BLEP
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(params: Value) -> Rack264Vco {
        Rack264Vco::build(
            &serde_json::from_value(json!({"id":"vco","kind":"rack264_vco","params":params}))
                .unwrap(),
            8192,
        )
        .unwrap()
    }
    fn step(model: &mut Rack264Vco, inputs: &[PortFrame; 4], mask: u16) -> [PortFrame; 4] {
        let mut outputs = [PortFrame::default(); 4];
        model.process(inputs, mask, &mut outputs);
        outputs
    }
    #[test]
    fn curved_waves_and_source_phase_cap_are_distinct_from_newer_stock() {
        for (phase, expected) in [
            (0.0, [0.0, 9.0 / 16.0, 9.0 / 16.0]),
            (0.25, [1.0, 1.0, 7.0 / 8.0]),
            (0.5, [0.0, -9.0 / 16.0, -1.0]),
            (0.75, [-1.0, -1.0, -1.0 / 56.0]),
        ] {
            for (output, value) in expected.into_iter().enumerate() {
                assert!((wave(output, phase) - value).abs() < 1e-6);
            }
        }
        let mut voice = Voice {
            frequency: 1.0,
            ..Default::default()
        };
        voice.process(1.0, 100.0, false, None, true);
        assert_eq!(voice.phase, 0.35);
        // Sine/saw/triangle are unfiltered; square alone receives highpass +0.95.
        assert_eq!(voice.values[0], wave(0, voice.phase));
        assert_eq!(voice.values[1], wave(1, voice.phase));
        assert_ne!(voice.values[3], voice.square);
    }
    #[test]
    fn source_sync_order_keeps_full_current_frame_in_soft_mode() {
        let mut soft = Voice {
            phase: 0.2,
            frequency: 0.2,
            last_sync: -1.0,
            ..Default::default()
        };
        soft.process(1.0, 100.0, true, Some(1.0), true);
        assert_eq!(soft.phase, 0.4);
        assert_eq!(soft.direction, -1.0);
        soft.process(1.0, 100.0, true, Some(1.0), true);
        assert_eq!(soft.phase, 0.2);
        let mut hard = Voice {
            phase: 0.6,
            frequency: 0.2,
            last_sync: -1.0,
            square: -1.0,
            ..Default::default()
        };
        hard.process(1.0, 100.0, false, Some(1.0), true);
        assert_eq!(hard.phase, 0.1);
        assert_eq!(hard.square, 1.0);
        assert!(hard
            .tails
            .iter()
            .all(|tail| tail.iter().any(|value| *value != 0.0)));
        let mut hidden = Voice {
            phase: 0.6,
            frequency: 0.2,
            last_sync: -1.0,
            square: -1.0,
            ..Default::default()
        };
        hidden.process(1.0, 100.0, false, Some(1.0), false);
        assert_eq!(hidden.phase, 0.1);
        assert_eq!(hidden.square, -1.0);
        assert_eq!(hidden.tails, [[0.0; LENGTH]; 4]);
    }
    #[test]
    fn stationary_pwm_latch_and_strict_blep_endpoints_follow_source() {
        let mut voice = Voice {
            phase: 0.5,
            width: 0.5,
            ..Default::default()
        };
        voice.process(1.0, 100.0, false, None, true);
        // 0/0 crossing becomes p=0 through scalar Rack fmin/fmax clamping.
        assert_eq!(voice.square, -1.0);
        assert!(voice.tails[3].iter().any(|value| *value != 0.0));
        let mut voice = Voice::default();
        voice.insert(0, -1.0, 2.0);
        assert_eq!(voice.tails[0], [0.0; LENGTH]);
        voice.insert(0, 0.0, 2.0);
        assert_eq!(voice.tails[0][0], 2.0 * (-1.0 + SYNC_BLEP[0]));
        voice.insert(0, f32::from_bits((-1.0_f32).to_bits() - 1), 1.0);
        assert!(voice.tails[0].iter().all(|v| v.is_finite()));
    }
    #[test]
    fn disconnected_consumers_do_not_freeze_internal_runtime() {
        let mut connected = model(json!({"soft_sync":1,"pwm_amount":1}));
        let mut disconnected = model(json!({"soft_sync":1,"pwm_amount":1}));
        let mut inputs = [PortFrame::default(); 4];
        for frame in 0..80 {
            inputs[2] = PortFrame::mono(if frame % 8 < 4 { -1.0 } else { 1.0 });
            inputs[3] = PortFrame::mono(if frame % 6 < 3 { -4.0 } else { 4.0 });
            step(&mut connected, &inputs, 15);
            let output = step(&mut disconnected, &inputs, 0);
            assert!(output
                .iter()
                .all(|f| f.channels == 0 && f.values == [0.0; MAX_CHANNELS]));
            assert_eq!(connected.snapshot(), disconnected.snapshot());
        }
        assert_eq!(
            step(&mut connected, &inputs, 15),
            step(&mut disconnected, &inputs, 15)
        );
    }
    #[test]
    fn active_hidden_lanes_advance_and_inactive_quartets_retain_history() {
        let mut model = model(json!({}));
        let mut inputs = [PortFrame::default(); 4];
        inputs[0].channels = 5;
        inputs[2] = PortFrame::mono(-1.0);
        step(&mut model, &inputs, 15);
        let before = model.snapshot();
        assert!(before.phases[7] > 0.0);
        assert_eq!(before.phases[8], 0.0);
        inputs[0].channels = 1;
        for _ in 0..10 {
            step(&mut model, &inputs, 0);
        }
        let after = model.snapshot();
        assert_ne!(before.phases[3], after.phases[3]);
        assert_eq!(before.phases[4], after.phases[4]);
        assert_eq!(before.filter_output[4], after.filter_output[4]);
        assert_eq!(before.buffer_indices[1], after.buffer_indices[1]);
        inputs[0].channels = 5;
        inputs[2] = PortFrame::mono(1.0);
        step(&mut model, &inputs, 15);
        assert_eq!(model.voices[4].last_sync, 1.0);
        assert!(model.voices[4].phase < before.phases[4]);
    }
    #[test]
    fn initialize_preserves_state_and_adoption_keeps_receiving_controls() {
        let mut old = model(json!({}));
        for _ in 0..19 {
            step(&mut old, &[PortFrame::default(); 4], 15);
        }
        let before = old.snapshot();
        old.initialize_state();
        assert_eq!(old.snapshot(), before);
        let mut next = model(json!({"frequency":12,"linear_fm":1}));
        next.adopt(&old);
        assert_eq!(next.snapshot(), before);
        let phase = next.voices[0].phase;
        step(&mut next, &[PortFrame::default(); 4], 0);
        assert_eq!(next.voices[0].phase, phase + 2.0 * C4 / 8192.0);
        let node = serde_json::from_value(json!({"id":"vco","kind":"rack264_vco"})).unwrap();
        let mut other_rate = Rack264Vco::build(&node, 48000).unwrap();
        other_rate.adopt(&old);
        assert_eq!(other_rate.voices[0].phase, 0.0);
    }
    #[test]
    fn boolean_frequency_and_unknown_controls_are_rejected() {
        for params in [
            json!({"linear_fm":0.5}),
            json!({"soft_sync":-1}),
            json!({"frequency":54.01}),
            json!({"frequency":-54.01}),
            json!({"pulse_width":0}),
            json!({"unknown":1}),
        ] {
            let node =
                serde_json::from_value(json!({"id":"vco","kind":"rack264_vco","params":params}))
                    .unwrap();
            assert!(Rack264Vco::build(&node, 48000).is_err());
        }
        assert!(model(json!({"frequency":54})).frequency == 4.5);
        assert!(model(json!({"frequency":-54})).frequency == -4.5);
    }
    #[test]
    fn finite_safety_extension_and_all_widths_masks_remain_bounded() {
        let values = [
            f32::MAX,
            -f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            0.0,
            -0.0,
            1e10,
            -1e10,
            1.0,
            -1.0,
            128.0,
            -128.0,
            256.0,
            -256.0,
            0.5,
        ];
        for linear in [0, 1] {
            let mut model = model(json!({"linear_fm":linear,"fm_amount":1,"pwm_amount":1}));
            for width in 0..=16 {
                for mask in 0..16 {
                    let mut inputs = [PortFrame {
                        channels: 16,
                        values,
                    }; 4];
                    inputs[0].channels = width;
                    let output = step(&mut model, &inputs, mask);
                    for (index, frame) in output.iter().enumerate() {
                        assert_eq!(
                            frame.channels,
                            if mask & (1 << index) == 0 {
                                0
                            } else {
                                width.max(1)
                            }
                        );
                        assert!(frame.values.iter().all(|v| v.is_finite()));
                    }
                    assert!(model.voices.iter().all(|v| v.phase.is_finite()
                        && v.phase >= 0.0
                        && v.phase < 1.0
                        && v.filter_output.is_finite()));
                    assert!(model
                        .voices
                        .iter()
                        .flat_map(|v| v.tails.iter().flatten())
                        .all(|v| v.is_finite()));
                }
            }
        }
    }
    #[test]
    fn phase_indicator_uses_sdk_sine_and_sixteen_frame_divider() {
        for phase in [0.0, 0.125, 0.25, 0.5, 0.75, 0.999] {
            assert!((phase_light(phase) - (std::f32::consts::TAU * phase).sin()).abs() < 2e-7);
        }
        let mut model = model(json!({"linear_fm":1,"soft_sync":1}));
        for _ in 0..15 {
            step(&mut model, &[PortFrame::default(); 4], 0);
        }
        assert_eq!(model.lamps, [0.0; 5]);
        step(&mut model, &[PortFrame::default(); 4], 0);
        assert_eq!(model.lamps[3..], [1.0, 1.0]);
        let mut inputs = [PortFrame::default(); 4];
        inputs[0].channels = 2;
        for _ in 0..16 {
            step(&mut model, &inputs, 0);
        }
        assert_eq!(model.lamps[..3], [0.0, 0.0, 1.0]);
    }
}
