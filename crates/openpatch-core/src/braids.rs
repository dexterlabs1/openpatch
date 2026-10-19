//! Original MIT-licensed Mutable Instruments Braids engine, with a bounded
//! polyphonic host and band-limited conversion from its native 96 kHz clock.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS};
use serde_json::{json, Value};
use std::{ffi::c_void, ptr::NonNull};
pub(crate) const INPUTS: [&str; 5] = ["pitch", "trigger", "fm", "timbre", "color"];
pub(crate) const MODELS: [&str; 47] = [
    "CSAW",
    "MORPH",
    "SAW/SQUARE",
    "SINE/TRI",
    "BUZZ",
    "SQUARE/SUB",
    "SAW/SUB",
    "SQUARE SYNC",
    "SAW SYNC",
    "TRIPLE SAW",
    "TRIPLE SQUARE",
    "TRIPLE TRIANGLE",
    "TRIPLE SINE",
    "RING MOD",
    "SWARM",
    "COMB",
    "TOY",
    "FILTER LP",
    "FILTER PEAK",
    "FILTER BP",
    "FILTER HP",
    "VOSIM",
    "VOWEL",
    "FOF VOWEL",
    "HARMONICS",
    "FM",
    "FEEDBACK FM",
    "CHAOTIC FM",
    "PLUCK",
    "BOW",
    "BLOW",
    "FLUTE",
    "BELL",
    "DRUM",
    "KICK",
    "CYMBAL",
    "SNARE",
    "WAVETABLE",
    "WAVE MAP",
    "WAVE LINE",
    "PARAPHONIC",
    "FILTERED NOISE",
    "TWIN PEAKS",
    "CLOCKED NOISE",
    "GRAINS",
    "PARTICLES",
    "DIGITAL MOD",
];
pub(crate) fn catalog() -> Value {
    json!({"kind":"braids","title":"BRAIDS","model":"functional","inputs":INPUTS,
    "description":"Original Mutable Instruments Braids firmware synthesis engine by Emilie Gillet (MIT): 47 macro-oscillator models, including feedback FM, formants, scanned wavetables, swarm, comb and physical models. Native 96 kHz DSP with band-limited host sample-rate conversion; up to 16 independent voices. Original installed Audible Instruments panel artwork. This is a firmware DSP port, not the Rack binary or complete hardware firmware.",
    "params":{
        "model":{"default":37,"min":0,"max":46,"step":1,"values":(0..47).collect::<Vec<_>>(),"labels":MODELS.to_vec()},
        "frequency":{"default":261.62558,"min":8,"max":16000,"unit":"Hz","sample_rate_max_ratio":0.4},
        "fine":{"default":0,"min":-1,"max":1,"unit":"semitones"},
        "timbre":{"default":0.5,"min":0,"max":1},"color":{"default":0.5,"min":0,"max":1},
        "fm_amount":{"default":0,"min":-1,"max":1},"timbre_amount":{"default":1,"min":-1,"max":1},
        "level":{"default":0.8,"min":0,"max":1}
    }})
}
extern "C" {
    fn openpatch_braids_new(seed: u32) -> *mut c_void;
    fn openpatch_braids_free(voice: *mut c_void);
    fn openpatch_braids_reset(voice: *mut c_void, seed: u32);
    fn openpatch_braids_copy(destination: *mut c_void, source: *const c_void);
    fn openpatch_braids_render(
        voice: *mut c_void,
        model: i32,
        pitch: i32,
        timbre: i32,
        color: i32,
        strike: i32,
        output: *mut i16,
    );
}
#[derive(Debug)]
struct Firmware(NonNull<c_void>);
// Each firmware instance has exclusive ownership; its RNG belongs to that voice. No shared mutable DSP state.
unsafe impl Send for Firmware {}
impl Firmware {
    fn new(seed: u32) -> Result<Self, String> {
        // SAFETY: constructor allocates and initializes one opaque firmware instance.
        NonNull::new(unsafe { openpatch_braids_new(seed) })
            .map(Self)
            .ok_or_else(|| "Unable to allocate Braids voice".into())
    }
}
impl Drop for Firmware {
    fn drop(&mut self) {
        // SAFETY: this instance uniquely owns the matching C++ allocation.
        unsafe {
            openpatch_braids_free(self.0.as_ptr());
        }
    }
}
#[derive(Debug)]
struct Voice {
    firmware: Firmware,
    block: [i16; 24],
    cursor: usize,
    history: [f32; 64],
    head: usize,
    phase: f64,
    controls: [f32; 3],
    gate: bool,
    strike: bool,
    dc_in: f32,
    dc_out: f32,
    seed: u32,
}
impl Voice {
    fn new(seed: u32) -> Result<Self, String> {
        Ok(Self {
            firmware: Firmware::new(seed)?,
            block: [0; 24],
            cursor: 24,
            history: [0.0; 64],
            head: 0,
            phase: 0.0,
            controls: [0.0; 3],
            gate: false,
            strike: false,
            dc_in: 0.0,
            dc_out: 0.0,
            seed,
        })
    }
    fn reset(&mut self) {
        // SAFETY: resets the already allocated, exclusively owned firmware object.
        unsafe {
            openpatch_braids_reset(self.firmware.0.as_ptr(), self.seed);
        }
        self.block.fill(0);
        self.cursor = 24;
        self.history.fill(0.0);
        self.head = 0;
        self.phase = 0.0;
        self.controls = [0.0; 3];
        self.gate = false;
        self.strike = false;
        self.dc_in = 0.0;
        self.dc_out = 0.0;
    }
    fn adopt(&mut self, old: &Self) {
        // SAFETY: both instances are valid, disjoint allocations of the same audited
        // scalar/inline-array firmware type. Copy performs no allocation or I/O.
        unsafe {
            openpatch_braids_copy(self.firmware.0.as_ptr(), old.firmware.0.as_ptr());
        }
        self.block = old.block;
        self.cursor = old.cursor;
        self.history = old.history;
        self.head = old.head;
        self.phase = old.phase;
        self.controls = old.controls;
        self.gate = old.gate;
        self.strike = old.strike;
        self.dc_in = old.dc_in;
        self.dc_out = old.dc_out;
    }
}
#[derive(Debug)]
pub(crate) struct Braids {
    rate: u32,
    model: i32,
    base_pitch: f32,
    timbre: f32,
    color: f32,
    level: f32,
    amounts: [f32; 2],
    ratio: f64,
    smooth: f32,
    dc: f32,
    kernels: Box<[[f32; 64]; 32]>,
    voices: Vec<Voice>,
    active: usize,
}
fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}
fn cv(input: &PortFrame, ch: usize, normal: f32) -> f32 {
    if input.channels == 0 {
        normal
    } else {
        finite(input.channel(if input.channels == 1 { 0 } else { ch }))
    }
}
impl Braids {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(8000..=384000).contains(&rate) {
            return Err("Braids requires 8000..384000 Hz".into());
        }
        for key in node.params.keys() {
            if ![
                "model",
                "frequency",
                "fine",
                "timbre",
                "color",
                "fm_amount",
                "timbre_amount",
                "level",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("unknown Braids parameter {key}"));
            }
        }
        let model = parameter(node, "model", 37.0, 0.0, 46.0)?;
        if model.fract() != 0.0 {
            return Err("Braids model must be an integer".into());
        }
        let frequency = parameter(node, "frequency", 261.62558, 8.0, 16000.0)?;
        let cutoff = (f64::from(rate) / 96000.0 * 0.46).min(0.46);
        let kernels = Box::new(std::array::from_fn(|phase| {
            let mut k = std::array::from_fn(|i| {
                let x = i as f64 - 31.5 + phase as f64 / 32.0;
                let sinc = if x.abs() < 1e-10 {
                    2.0 * cutoff
                } else {
                    (std::f64::consts::TAU * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let window = 0.42 - 0.5 * (std::f64::consts::TAU * i as f64 / 63.0).cos()
                    + 0.08 * (2.0 * std::f64::consts::TAU * i as f64 / 63.0).cos();
                (sinc * window) as f32
            });
            let sum: f32 = k.iter().sum();
            for a in &mut k {
                *a /= sum;
            }
            k
        }));
        Ok(Self {
            rate,
            model: model as i32,
            base_pitch: 69.0
                + 12.0 * (frequency / 440.0).log2()
                + parameter(node, "fine", 0.0, -1.0, 1.0)?,
            timbre: parameter(node, "timbre", 0.5, 0.0, 1.0)?,
            color: parameter(node, "color", 0.5, 0.0, 1.0)?,
            level: parameter(node, "level", 0.8, 0.0, 1.0)?,
            amounts: [
                parameter(node, "fm_amount", 0.0, -1.0, 1.0)?,
                parameter(node, "timbre_amount", 1.0, -1.0, 1.0)?,
            ],
            ratio: 96000.0 / f64::from(rate),
            smooth: (1.0 - (-1.0 / (0.004 * f64::from(rate))).exp()) as f32,
            dc: (-std::f64::consts::TAU * 5.0 / f64::from(rate)).exp() as f32,
            kernels,
            voices: (0..MAX_CHANNELS)
                .map(|i| Voice::new(0xBA1D5000 + i as u32))
                .collect::<Result<_, _>>()?,
            active: 0,
        })
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let get = |i| inputs.get(i).copied().unwrap_or_default();
        let p = get(0);
        let trig = get(1);
        let fm = get(2);
        let timbre = get(3);
        let color = get(4);
        let level = get(5);
        let width = usize::from(p.channels.max(trig.channels).max(1)).min(MAX_CHANNELS);
        for voice in &mut self.voices[width..self.active.max(width)] {
            voice.reset();
        }
        outputs.fill(PortFrame::default());
        if outputs.is_empty() {
            return;
        }
        outputs[0].channels = width as u8;
        for ch in 0..width {
            let v = &mut self.voices[ch];
            let gate = cv(&trig, ch, 0.0);
            if !v.gate && gate >= 1.0 {
                v.gate = true;
                v.strike = true;
            } else if v.gate && gate <= 0.1 {
                v.gate = false;
            }
            let target = [
                (self.base_pitch + 12.0 * (cv(&p, ch, 0.0) + cv(&fm, ch, 0.0) * self.amounts[0]))
                    .clamp(0.0, 127.99),
                (self.timbre + cv(&timbre, ch, 0.0) * self.amounts[1] / 5.0).clamp(0.0, 1.0),
                (self.color + cv(&color, ch, 0.0) / 5.0).clamp(0.0, 1.0),
            ];
            if ch >= self.active {
                v.controls = target;
            }
            for (x, t) in v.controls.iter_mut().zip(target) {
                *x += (t - *x) * self.smooth;
            }
            v.phase += self.ratio;
            while v.phase >= 1.0 {
                if v.cursor == 24 {
                    // SAFETY: valid voice, bounded controls, and an output array of
                    // exactly the 24 samples required by the original firmware.
                    unsafe {
                        openpatch_braids_render(
                            v.firmware.0.as_ptr(),
                            self.model,
                            (v.controls[0] * 128.0) as i32,
                            (v.controls[1] * 32767.0) as i32,
                            (v.controls[2] * 32767.0) as i32,
                            i32::from(v.strike),
                            v.block.as_mut_ptr(),
                        );
                    }
                    v.strike = false;
                    v.cursor = 0;
                }
                v.history[v.head] = v.block[v.cursor] as f32 / 32768.0;
                v.cursor += 1;
                v.head = (v.head + 1) % 64;
                v.phase -= 1.0;
            }
            let kernel = &self.kernels[(v.phase * 32.0) as usize];
            let raw: f32 = kernel
                .iter()
                .enumerate()
                .map(|(i, k)| v.history[(v.head + 63 - i) % 64] * k)
                .sum();
            let out = raw - v.dc_in + self.dc * v.dc_out;
            v.dc_in = raw;
            v.dc_out = out;
            outputs[0].values[ch] =
                out * 5.0 * self.level * (cv(&level, ch, 10.0) / 10.0).clamp(0.0, 1.0);
        }
        self.active = width;
    }
    pub(crate) fn adopt(&mut self, old: &Self, trigger_same: bool) {
        if self.rate != old.rate || self.model != old.model {
            return;
        }
        self.active = old.active;
        for (a, b) in self.voices.iter_mut().zip(&old.voices) {
            a.adopt(b);
            if !trigger_same {
                a.gate = false;
                a.strike = false;
            }
        }
    }
    pub(crate) fn reset_state(&mut self) {
        for v in &mut self.voices {
            v.reset();
        }
        self.active = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn voice(model: usize, rate: u32) -> Braids {
        let n:Node=serde_json::from_value(json!({"id":"b","kind":"braids","params":{"model":model,"frequency":220,"timbre":0.0,"color":0.0}})).unwrap();
        Braids::build(&n, rate).unwrap()
    }
    #[test]
    fn original_sine_pitch_survives_sample_rate_conversion() {
        for rate in [44100, 48000, 96000] {
            let mut b = voice(3, rate);
            let mut out = [PortFrame::default()];
            let mut previous = 0.;
            let mut crossings = 0;
            let mut energy = 0.;
            for i in 0..rate * 2 {
                b.process(&[], &mut out);
                let x = out[0].values[0];
                if i > rate / 2 {
                    if x >= 0. && previous < 0. {
                        crossings += 1;
                    }
                    energy += x * x;
                }
                previous = x;
            }
            assert!(
                (crossings as f64 / 1.5 - 220.).abs() < 1.,
                "rate {rate}, crossings {crossings}"
            );
            assert!(energy > 100.);
        }
    }
    #[test]
    fn every_original_model_is_finite_and_state_adoption_is_continuous() {
        for model in 0..47 {
            let mut a = voice(model, 48000);
            let mut b = voice(model, 48000);
            let mut x = [PortFrame::default()];
            let mut y = x;
            for _ in 0..2048 {
                a.process(&[], &mut x);
                assert!(x[0].values[0].is_finite());
                assert!(x[0].values[0].abs() < 8.0);
            }
            b.adopt(&a, true);
            for _ in 0..64 {
                a.process(&[], &mut x);
                b.process(&[], &mut y);
                assert_eq!(x, y, "model {model}");
            }
        }
    }
    #[test]
    fn timbre_and_color_change_spectra_without_losing_independent_polyphony() {
        let mut a = voice(37, 48000);
        let mut b = voice(37, 48000);
        b.timbre = 0.8;
        b.color = 0.7;
        let pitch = PortFrame {
            channels: 4,
            values: std::array::from_fn(|i| i as f32 / 12.),
        };
        let mut x = [PortFrame::default()];
        let mut y = x;
        let mut difference = 0.;
        for _ in 0..4096 {
            a.process(&[pitch], &mut x);
            b.process(&[pitch], &mut y);
            difference += (x[0].values[0] - y[0].values[0]).abs();
        }
        assert_eq!(x[0].channels, 4);
        assert!(difference > 100.);
        assert_ne!(x[0].values[0], x[0].values[1]);
        a.process(&[PortFrame::mono(0.)], &mut x);
        assert_eq!(x[0].channels, 1);
        assert_eq!(x[0].values[1], 0.);
    }
}
