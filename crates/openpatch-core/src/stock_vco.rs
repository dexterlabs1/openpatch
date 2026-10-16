//! Original scalar Rust realization of the pinned stock VCO behavioral contract.
//! Mathematical methods: Brandt minimum-phase step/ramp reconstruction, radix-2
//! FFT/real cepstrum, polynomial sine, and a one-pole DC blocker. The observable
//! arithmetic order and mathematical coefficients are specified by Fundamental
//! 10dd0160 VCO; no upstream implementation, generated table, or art is embedded.
use crate::{parameter, ModuleLightState, Node, PortFrame, MAX_CHANNELS};
use serde_json::{json, Value};
use std::sync::OnceLock;

pub(crate) const INPUTS: [&str; 4] = ["pitch", "fm", "sync", "pwm"];
pub(crate) const OUTPUTS: [&str; 4] = ["sine", "triangle", "saw", "square"];
const C4: f32 = 261.6256;
const LENGTH: usize = 32;
const OVERSAMPLE: usize = 16;
const FFT_SIZE: usize = LENGTH * OVERSAMPLE;

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_vco","title":"STOCK VCO","model":"functional","description":"Stock-style audio oscillator with prepared minimum-phase step/ramp reconstruction, within-frame hard/soft sync, four consumer-gated outputs and observed phase/mode lamps. Existing multi_vco remains independent.","inputs":INPUTS,"params":{
        "frequency":{"default":0.0,"min":-76.0,"max":76.0,"unit":"semitones","description":"C4 × 2^(frequency/12) Hz before pitch/FM"},
        "pulse_width":{"default":0.5,"min":0.01,"max":0.99,"unit":"ratio"},
        "fm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Signed exponential/linear FM depth; Randomize disabled"},
        "pwm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"PWM/10 times signed depth; Randomize disabled"},
        "linear_fm":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1]},
        "soft_sync":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1]}
    }})
}
pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/VCO.cpp");
    module["polyphony"]["primary_input"] = json!("pitch");
    module["polyphony"]["version_2"] = json!("max(pitch width,1), at most16; all inputs mono broadcast and zero for missing short-poly lanes");
    module["polyphony"]["retired_voices"] = json!("active quartets advance all phase/sync lanes; only visible lanes insert corrections; connected output filters/tails advance whole quartets; wholly inactive groups freeze, without retirement reset");
    module["consumer_policy"] = json!("disconnected outputs report zero channels and zero host storage; their correction buffers and DC filters freeze; phase/sync and lamps continue; inactive raw output storage is excluded from reference voltage equivalence");
    module["frequency_policy"] = json!("f32 fifth-order exp2 approximation, C4 pitch, exponential or linear FM; frequency clamps0..sample_rate/2, phase delta0..0.49; nonfinite CV becomes0, finite CV has no arbitrary amplitude clamp; out-of-domain exponent safely saturates[-127,128]");
    module["reconstruction_policy"] = json!("prepared original Brandt real-cepstrum minimum-phase step and integrated ramp,16×32 samples; step and slope corrections for PWM/sync/crossings; polynomial sine; all four waves have a20Hz approximate one-pole DC blocker");
    module["light_names"] = json!([
        "phase_red",
        "phase_green",
        "phase_blue",
        "linear_fm",
        "soft_sync"
    ]);
    module["light_policy"] = json!("five primary components, every16 callbacks: mono negative/positive polynomial sine smoothed with30/s release, or poly blue; linear/soft mode lamps; bypass freezes runtime");
    module["initialization_policy"] = json!("source has no runtime reset override: Initialize restores controls through the host while preserving phase, sync, tails, filters, lamps and divider");
    module["state_adoption"] = json!("same identity/kind/sample rate preserves all bounded runtime; receiving controls win; changed sync routing preserves detector history; fresh/rate changes start fresh");
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
        y = y * x + coefficient;
    }
    f32::from_bits(integer << 23) * y
}
#[allow(clippy::excessive_precision)]
fn wave(wave: usize, phase: f32, width: f32) -> f32 {
    match wave {
        0 => {
            let t = 2.0 * phase - 1.0;
            let t2 = t * t;
            t * (t2 - 1.0)
                * (3.1415211942925003_f32
                    + t2 * (-2.0247734792732333_f32
                        + t2 * (0.51749274223813413_f32 + t2 * -0.063691093590695858_f32)))
        }
        1 => 1.0 - 4.0 * (phase - 0.25).abs().min(1.25 - phase),
        2 => {
            let x = phase + 0.5;
            2.0 * (x - x.trunc()) - 1.0
        }
        3 => {
            if phase < width {
                1.0
            } else {
                -1.0
            }
        }
        _ => unreachable!(),
    }
}

// Independently authored general radix-2 transform, shared mathematical method
// with OpenPatch's original multi_vco. Coefficients are prepared off callback.
#[derive(Clone, Copy, Default)]
struct Complex {
    re: f64,
    im: f64,
}
fn fft(data: &mut [Complex; FFT_SIZE], inverse: bool) {
    let mut reverse = 0;
    for i in 1..FFT_SIZE {
        let mut bit = FFT_SIZE >> 1;
        while reverse & bit != 0 {
            reverse ^= bit;
            bit >>= 1;
        }
        reverse ^= bit;
        if i < reverse {
            data.swap(i, reverse);
        }
    }
    let mut length = 2;
    while length <= FFT_SIZE {
        let angle = (if inverse { 1.0 } else { -1.0 }) * 2.0 * std::f64::consts::PI / length as f64;
        let rotation = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for start in (0..FFT_SIZE).step_by(length) {
            let mut factor = Complex { re: 1.0, im: 0.0 };
            for k in 0..length / 2 {
                let a = data[start + k];
                let b = data[start + k + length / 2];
                let b = Complex {
                    re: b.re * factor.re - b.im * factor.im,
                    im: b.re * factor.im + b.im * factor.re,
                };
                data[start + k] = Complex {
                    re: a.re + b.re,
                    im: a.im + b.im,
                };
                data[start + k + length / 2] = Complex {
                    re: a.re - b.re,
                    im: a.im - b.im,
                };
                factor = Complex {
                    re: factor.re * rotation.re - factor.im * rotation.im,
                    im: factor.re * rotation.im + factor.im * rotation.re,
                };
            }
        }
        length *= 2;
    }
    if inverse {
        for x in data {
            x.re /= FFT_SIZE as f64;
            x.im /= FFT_SIZE as f64;
        }
    }
}
#[derive(Debug)]
struct Kernel {
    step: [[f32; LENGTH + 4]; OVERSAMPLE],
    ramp: [[f32; LENGTH + 4]; OVERSAMPLE],
}
static KERNEL: OnceLock<Kernel> = OnceLock::new();
impl Kernel {
    fn prepare() -> Self {
        let mut data = [Complex::default(); FFT_SIZE];
        for (i, x) in data.iter_mut().enumerate() {
            let time = i as f64 / OVERSAMPLE as f64 - 16.0;
            let angle = std::f64::consts::PI * time;
            let sinc = if time == 0.0 {
                1.0
            } else {
                angle.sin() / angle
            };
            let p = i as f64 / (FFT_SIZE - 1) as f64;
            let window = 0.35875 - 0.48829 * (2.0 * std::f64::consts::PI * p).cos()
                + 0.14128 * (4.0 * std::f64::consts::PI * p).cos()
                - 0.01168 * (6.0 * std::f64::consts::PI * p).cos();
            x.re = sinc * window;
        }
        fft(&mut data, false);
        for x in &mut data {
            x.re = x.re.hypot(x.im).ln();
            x.im = 0.0;
        }
        fft(&mut data, true);
        for x in &mut data[1..FFT_SIZE / 2] {
            x.re *= 2.0;
            x.im *= 2.0;
        }
        data[FFT_SIZE / 2..].fill(Complex::default());
        fft(&mut data, false);
        for x in &mut data {
            let a = x.re.exp();
            *x = Complex {
                re: a * x.im.cos(),
                im: a * x.im.sin(),
            };
        }
        fft(&mut data, true);
        let mut impulse = [0.0_f32; FFT_SIZE];
        let mut sum = 0.0_f64;
        for (out, x) in impulse.iter_mut().zip(data) {
            *out = sum as f32;
            sum += x.re / OVERSAMPLE as f64;
        }
        for out in &mut impulse {
            *out = (f64::from(*out) / sum) as f32;
            *out -= 1.0;
        }
        let mut integrated = [0.0_f32; FFT_SIZE];
        sum = 0.0;
        for (out, x) in integrated.iter_mut().zip(impulse) {
            *out = sum as f32;
            sum += f64::from(x / OVERSAMPLE as f32);
        }
        for (i, out) in integrated.iter_mut().enumerate() {
            *out = (f64::from(*out) - f64::from(i as f32 / FFT_SIZE as f32) * sum) as f32;
        }
        let mut result = Self {
            step: [[0.0; LENGTH + 4]; OVERSAMPLE],
            ramp: [[0.0; LENGTH + 4]; OVERSAMPLE],
        };
        for o in 0..OVERSAMPLE {
            for z in 0..LENGTH {
                result.step[o][z] = impulse[z * OVERSAMPLE + o];
                result.ramp[o][z] = integrated[z * OVERSAMPLE + o];
            }
        }
        assert!(result
            .step
            .iter()
            .flatten()
            .chain(result.ramp.iter().flatten())
            .all(|v| v.is_finite()));
        result
    }
    fn insert(
        &self,
        tail: &mut [f32; LENGTH],
        cursor: usize,
        time: f32,
        magnitude: f32,
        slope: bool,
    ) {
        if !(time > 0.0 && time <= 1.0) || !magnitude.is_finite() {
            return;
        }
        let position = (1.0 - time) * OVERSAMPLE as f32;
        // Finite safety extension when a positive subnormal time rounds to0.
        let whole = (position as usize).min(OVERSAMPLE);
        let fraction = position - whole as f32;
        let table = if slope { &self.ramp } else { &self.step };
        let o = whole % OVERSAMPLE;
        let shift = whole / OVERSAMPLE;
        for z in 0..LENGTH {
            let first = table[o][z + shift];
            let next = table[(o + 1) % OVERSAMPLE][z + shift + (o + 1) / OVERSAMPLE];
            let correction = (first + fraction * (next - first)) * magnitude;
            tail[(cursor + z) % LENGTH] += correction;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    phase: f32,
    direction: f32,
    last_sync: f32,
    last_square: f32,
    filters: [f32; 4],
    tails: [[f32; LENGTH]; 4],
    cursors: [u8; 4],
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            phase: 0.0,
            direction: 1.0,
            last_sync: 0.0,
            last_square: 1.0,
            filters: [0.0; 4],
            tails: [[0.0; LENGTH]; 4],
            cursors: [0; 4],
        }
    }
}
fn crossing(threshold: f32, start: f32, end: f32, begin: f32, finish: f32) -> f32 {
    let delta = end - start;
    let diff = threshold - start;
    let boundary = threshold
        - if delta >= 0.0 {
            diff.floor()
        } else {
            diff.ceil()
        };
    begin + (boundary - start) / delta * (finish - begin)
}
impl Voice {
    fn correction(
        &mut self,
        kernel: &Kernel,
        output: usize,
        time: f32,
        magnitude: f32,
        slope: bool,
        visible: bool,
    ) {
        if visible {
            kernel.insert(
                &mut self.tails[output],
                usize::from(self.cursors[output]),
                time,
                magnitude,
                slope,
            );
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn segment(
        &mut self,
        kernel: &Kernel,
        start: f32,
        end: f32,
        begin: f32,
        finish: f32,
        width: f32,
        delta: f32,
        mask: u16,
        visible: bool,
    ) {
        let direction = self.direction;
        let mut edge = |wave: usize, threshold: f32, magnitude: f32, slope: bool| {
            if mask & (1 << wave) == 0 {
                return;
            }
            let time = crossing(threshold, start, end, begin, finish);
            if begin < time && time <= finish {
                self.correction(kernel, wave, time, magnitude, slope, visible);
            }
        };
        edge(3, 1.0, 2.0 * direction, false);
        edge(3, width, -2.0 * direction, false);
        edge(2, 0.5, -2.0 * direction, false);
        edge(1, 0.25, -8.0 * delta * direction, true);
        edge(1, 0.75, 8.0 * delta * direction, true);
    }
    #[allow(clippy::too_many_arguments)]
    fn process(
        &mut self,
        kernel: &Kernel,
        frequency: f32,
        width: f32,
        sync: Option<f32>,
        soft: bool,
        sample_time: f32,
        alpha: f32,
        mask: u16,
        visible: bool,
    ) -> [f32; 4] {
        let mut delta = (frequency * sample_time).clamp(0.0, 0.49);
        if soft {
            delta *= self.direction;
        } else {
            self.direction = 1.0;
        }
        let start = self.phase;
        let width = width.clamp(0.01, 0.99);
        if mask & 8 != 0 {
            let changed = wave(3, start, width) - self.last_square;
            if changed != 0.0 {
                self.correction(kernel, 3, 1e-6, changed, false, visible);
            }
        }
        let sync_time = sync.and_then(|value| {
            let difference = value - self.last_sync;
            let at = -self.last_sync / difference;
            self.last_sync = value;
            (0.0 < at && at <= 1.0 && difference >= 0.0).then_some(at)
        });
        if let Some(at) = sync_time {
            let unwrapped = start + delta * at;
            self.segment(
                kernel, start, unwrapped, 0.0, at, width, delta, mask, visible,
            );
            let phase = unwrapped - unwrapped.floor();
            if soft {
                if mask & 4 != 0 {
                    self.correction(kernel, 2, at, -4.0 * delta, true, visible);
                }
                if mask & 2 != 0 {
                    let slope = if 0.25 < phase && phase < 0.75 {
                        8.0
                    } else {
                        -8.0
                    };
                    self.correction(kernel, 1, at, slope * delta, true, visible);
                }
                self.direction = -self.direction;
                delta = -delta;
                let end = phase + delta * (1.0 - at);
                self.segment(kernel, phase, end, at, 1.0, width, delta, mask, visible);
                self.phase = end;
            } else {
                if mask & 8 != 0 && phase >= width {
                    self.correction(kernel, 3, at, 2.0, false, visible);
                }
                for output in [2, 1, 0] {
                    if mask & (1 << output) != 0 {
                        self.correction(
                            kernel,
                            output,
                            at,
                            -wave(output, phase, width),
                            false,
                            visible,
                        );
                    }
                }
                if mask & 2 != 0 && 0.25 < phase && phase < 0.75 {
                    self.correction(kernel, 1, at, 8.0 * delta, true, visible);
                }
                let end = delta * (1.0 - at);
                self.segment(kernel, 0.0, end, at, 1.0, width, delta, mask, visible);
                self.phase = end;
            }
        } else {
            let end = start + delta;
            self.segment(kernel, start, end, 0.0, 1.0, width, delta, mask, visible);
            self.phase = end;
        }
        self.phase -= self.phase.floor();
        let mut result = [0.0; 4];
        for (output, value) in result.iter_mut().enumerate() {
            if mask & (1 << output) == 0 {
                continue;
            }
            let raw = wave(output, self.phase, width);
            if output == 3 {
                self.last_square = raw;
            }
            let cursor = usize::from(self.cursors[output]);
            let corrected = raw + self.tails[output][cursor];
            self.tails[output][cursor] = 0.0;
            self.cursors[output] = ((cursor + 1) % LENGTH) as u8;
            self.filters[output] += alpha * (corrected - self.filters[output]);
            *value = 5.0 * (corrected - self.filters[output]);
        }
        result
    }
}

/// Read-only actual production state. Correction tails are logical future32
/// samples; physical ring indices are separately observable per wave/quartet.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct StockVcoSnapshot {
    pub phases: [f32; MAX_CHANNELS],
    pub sync_directions: [f32; MAX_CHANNELS],
    pub last_sync: [f32; MAX_CHANNELS],
    pub last_square: [f32; MAX_CHANNELS],
    pub filter_state: [[f32; MAX_CHANNELS]; 4],
    pub correction_tails: [[[f32; LENGTH]; MAX_CHANNELS]; 4],
    pub buffer_indices: [[u8; 4]; 4],
    pub light_divider: u8,
    pub lights: [f32; 5],
}
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct StockVcoKernelSnapshot {
    pub impulse: [[f32; LENGTH]; OVERSAMPLE],
    pub ramp: [[f32; LENGTH]; OVERSAMPLE],
}
#[derive(Debug)]
pub(crate) struct StockVco {
    sample_time: f32,
    frequency_limit: f32,
    alpha: f32,
    frequency: f32,
    fm_amount: f32,
    pulse_width: f32,
    pwm_amount: f32,
    linear: bool,
    soft: bool,
    kernel: &'static Kernel,
    voices: [Voice; MAX_CHANNELS],
    divider: u8,
    lamps: [f32; 5],
}
impl StockVco {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("stock_vco sample rate must be8000..=192000 Hz".into());
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
        let w = std::f32::consts::TAU * (20.0 * sample_time).min(0.4);
        Ok(Self {
            sample_time,
            frequency_limit: sample_rate as f32 / 2.0,
            alpha: w / (1.0 + w),
            frequency: parameter(node, "frequency", 0.0, -76.0, 76.0)? / 12.0,
            fm_amount: parameter(node, "fm_amount", 0.0, -1.0, 1.0)?,
            pulse_width: parameter(node, "pulse_width", 0.5, 0.01, 0.99)?,
            pwm_amount: parameter(node, "pwm_amount", 0.0, -1.0, 1.0)?,
            linear: flag("linear_fm")?,
            soft: flag("soft_sync")?,
            kernel: KERNEL.get_or_init(Kernel::prepare),
            voices: [Voice::default(); MAX_CHANNELS],
            divider: 0,
            lamps: [0.0; 5],
        })
    }
    pub(crate) fn initialize_state(&mut self) { /* No source runtime-reset callback. */
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
            // Source-domain infinities cap at Nyquist. Finite extreme linear FM
            // can form Inf-Inf; the safety extension chooses a finite stopped phase.
            let frequency = if frequency.is_nan() {
                0.0
            } else {
                frequency.clamp(0.0, self.frequency_limit)
            };
            let width =
                self.pulse_width + finite(inputs[3].voice(lane, 0.0)) / 10.0 * self.pwm_amount;
            let sync = (inputs[2].channels != 0).then(|| finite(inputs[2].voice(lane, 0.0)));
            let result = self.voices[lane].process(
                self.kernel,
                frequency,
                width,
                sync,
                self.soft,
                self.sample_time,
                self.alpha,
                mask,
                lane < usize::from(channels),
            );
            if lane < usize::from(channels) {
                for (output, value) in outputs.iter_mut().zip(result) {
                    output.values[lane] = value;
                }
            }
        }
        self.divider += 1;
        if self.divider == 16 {
            self.divider = 0;
            if channels == 1 {
                let phase = wave(0, self.voices[0].phase, self.pulse_width);
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
    fn snapshot(&self) -> StockVcoSnapshot {
        StockVcoSnapshot {
            phases: std::array::from_fn(|i| self.voices[i].phase),
            sync_directions: std::array::from_fn(|i| self.voices[i].direction),
            last_sync: std::array::from_fn(|i| self.voices[i].last_sync),
            last_square: std::array::from_fn(|i| self.voices[i].last_square),
            filter_state: std::array::from_fn(|w| {
                std::array::from_fn(|i| self.voices[i].filters[w])
            }),
            correction_tails: std::array::from_fn(|w| {
                std::array::from_fn(|i| {
                    std::array::from_fn(|s| {
                        self.voices[i].tails[w]
                            [(usize::from(self.voices[i].cursors[w]) + s) % LENGTH]
                    })
                })
            }),
            buffer_indices: std::array::from_fn(|w| {
                std::array::from_fn(|g| self.voices[g * 4].cursors[w])
            }),
            light_divider: self.divider,
            lights: self.lamps,
        }
    }
}
/// Diagnostics of the same processor used by Engine; no second DSP model.
#[doc(hidden)]
#[derive(Debug)]
pub struct StockVcoProbe(StockVco);
impl StockVcoProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        StockVco::build(node, rate).map(Self)
    }
    pub fn step(&mut self, inputs: &[PortFrame; 4], mask: u16) -> [PortFrame; 4] {
        let mut outputs = [PortFrame::default(); 4];
        self.0.process(inputs, mask, &mut outputs);
        outputs
    }
    pub fn snapshot(&self) -> StockVcoSnapshot {
        self.0.snapshot()
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn adopt(&mut self, old: &Self) {
        self.0.adopt(&old.0);
    }
    pub fn kernel(&self) -> StockVcoKernelSnapshot {
        StockVcoKernelSnapshot {
            impulse: std::array::from_fn(|o| std::array::from_fn(|z| self.0.kernel.step[o][z])),
            ramp: std::array::from_fn(|o| std::array::from_fn(|z| self.0.kernel.ramp[o][z])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(params: Value) -> StockVco {
        StockVco::build(
            &serde_json::from_value(json!({"id":"vco","kind":"stock_vco","params":params}))
                .unwrap(),
            8192,
        )
        .unwrap()
    }
    fn step(model: &mut StockVco, inputs: &[PortFrame; 4], mask: u16) -> [PortFrame; 4] {
        let mut out = [PortFrame::default(); 4];
        model.process(inputs, mask, &mut out);
        out
    }
    #[test]
    fn prepared_step_and_ramp_have_source_sampling_origins_and_finite_padding() {
        let model = model(json!({}));
        let kernel = model.kernel;
        assert_eq!(kernel.step[0][0], -1.0);
        assert_eq!(kernel.ramp[0][0], 0.0);
        assert!(kernel
            .step
            .iter()
            .flatten()
            .chain(kernel.ramp.iter().flatten())
            .all(|v| v.is_finite()));
        assert!(kernel.step.iter().all(|row| row[LENGTH..] == [0.0; 4]));
        assert!(kernel.ramp.iter().all(|row| row[LENGTH..] == [0.0; 4]));
        assert!(kernel.ramp.iter().flatten().any(|v| v.abs() > 0.01));
        let snapshot = StockVcoProbe(model).kernel();
        assert_eq!(snapshot.impulse[3][7], kernel.step[3][7]);
    }
    #[test]
    fn landmarks_are_new_stock_waves_and_phase_uses_exact_sdk_c4() {
        for (phase, expected) in [
            (0.0, [0.0, 0.0, 0.0, 1.0]),
            (0.25, [1.0, 1.0, 0.5, 1.0]),
            (0.5, [0.0, 0.0, -1.0, -1.0]),
            (0.75, [-1.0, -1.0, -0.5, -1.0]),
        ] {
            for (i, target) in expected.into_iter().enumerate() {
                assert!((wave(i, phase, 0.5) - target).abs() < 1e-5);
            }
        }
        let mut model = model(json!({}));
        step(&mut model, &[PortFrame::default(); 4], 15);
        assert_eq!(model.voices[0].phase, C4 * (1.0 / 8192.0));
        assert_eq!(C4.to_bits(), 261.6256_f32.to_bits());
    }
    #[test]
    fn sync_splits_current_frame_for_both_modes_and_requires_positive_slope() {
        let kernel = KERNEL.get_or_init(Kernel::prepare);
        let run = |soft: bool, last: f32, value: f32| {
            let mut voice = Voice {
                phase: 0.2,
                last_sync: last,
                ..Default::default()
            };
            voice.process(kernel, 0.2, 0.5, Some(value), soft, 1.0, 0.0, 15, true);
            voice
        };
        let hard = run(false, -1.0, 1.0);
        assert!((hard.phase - 0.1).abs() < 1e-7);
        let soft = run(true, -1.0, 1.0);
        assert!((soft.phase - 0.2).abs() < 1e-7);
        assert_eq!(soft.direction, -1.0);
        let falling_zero = run(false, 1.0, 0.0);
        assert!((falling_zero.phase - 0.4).abs() < 1e-7);
        assert!((run(false, 0.0, 1.0).phase - 0.4).abs() < 1e-7);
        assert_eq!(hard.last_sync, 1.0);
        assert!(hard.tails.iter().flatten().any(|value| *value != 0.0));
    }
    #[test]
    fn pwm_can_reopen_at_zero_frequency_and_corrections_reject_zero_time() {
        let mut voice = Voice {
            phase: 0.4,
            last_square: -1.0,
            ..Default::default()
        };
        let kernel = KERNEL.get_or_init(Kernel::prepare);
        voice.process(kernel, 0.0, 0.8, None, false, 1.0, 0.0, 8, true);
        assert_eq!(voice.last_square, 1.0);
        assert!(voice.tails[3].iter().any(|v| *v != 0.0));
        let mut tail = [0.0; LENGTH];
        kernel.insert(&mut tail, 0, 0.0, 2.0, false);
        assert_eq!(tail, [0.0; LENGTH]);
        kernel.insert(&mut tail, 0, 1.0, 2.0, false);
        assert_eq!(tail[0], -2.0);
    }
    #[test]
    fn quartet_hidden_lanes_continue_whole_groups_freeze_and_regrowth_retains_state() {
        let mut model = model(json!({"fm_amount":1}));
        let mut inputs = [PortFrame::default(); 4];
        inputs[0].channels = 5;
        inputs[0].values[4] = 1.0;
        inputs[2] = PortFrame::mono(-1.0);
        step(&mut model, &inputs, 15);
        let before = model.snapshot();
        assert!(before.phases[7] > 0.0);
        assert_eq!(before.phases[8], 0.0);
        inputs[0].channels = 1;
        for _ in 0..5 {
            step(&mut model, &inputs, 15);
        }
        let shrunk = model.snapshot();
        assert_ne!(shrunk.phases[3], before.phases[3]);
        assert_eq!(shrunk.phases[4], before.phases[4]);
        assert_eq!(shrunk.last_sync[4], -1.0);
        inputs[0].channels = 5;
        inputs[2] = PortFrame::mono(1.0);
        step(&mut model, &inputs, 15);
        assert_eq!(model.voices[4].last_sync, 1.0);
        assert!(model.voices[4].phase < before.phases[4]);
        // Invisible lanes advance a connected wave/filter but cannot inject new correction events.
        let mut hidden = Voice {
            phase: 0.4,
            last_square: -1.0,
            ..Default::default()
        };
        hidden.process(model.kernel, 0.0, 0.8, None, false, 1.0, 0.1, 8, false);
        assert_eq!(hidden.tails[3], [0.0; LENGTH]);
        assert_eq!(hidden.last_square, 1.0);
        assert_ne!(hidden.filters[3], 0.0);
    }
    #[test]
    fn consumers_freeze_only_their_tails_filters_and_latches_and_lights_keep_source_sign() {
        let mut model = model(json!({"linear_fm":1,"soft_sync":1}));
        let inputs = [PortFrame::default(); 4];
        for _ in 0..16 {
            step(&mut model, &inputs, 15);
        }
        let before = model.snapshot();
        for _ in 0..16 {
            let out = step(&mut model, &inputs, 0);
            assert!(out.iter().all(|f| f.channels == 0 && f.values == [0.0; 16]));
        }
        let after = model.snapshot();
        assert_ne!(after.phases, before.phases);
        assert_eq!(after.filter_state, before.filter_state);
        assert_eq!(after.correction_tails, before.correction_tails);
        assert_eq!(after.buffer_indices, before.buffer_indices);
        assert_eq!(after.last_square, before.last_square);
        assert_eq!(after.lights[3..], [1.0, 1.0]);
        assert_eq!(after.light_divider, 0);
        // Source Light smoothing accepts a signed target; it is not an LED-domain clamp.
        model.voices[0].phase = 0.25;
        model.divider = 15;
        model.frequency = -76.0 / 12.0;
        step(&mut model, &inputs, 0);
        assert!(model.lamps[0] < before.lights[0]);
    }
    #[test]
    fn runtime_initialize_is_noop_adoption_preserves_runtime_and_receiving_controls_win() {
        let mut old = model(json!({}));
        for _ in 0..19 {
            step(&mut old, &[PortFrame::default(); 4], 15);
        }
        let before = old.snapshot();
        old.initialize_state();
        assert_eq!(old.snapshot(), before);
        let mut next = model(json!({"frequency":12,"soft_sync":1}));
        next.adopt(&old);
        assert_eq!(next.snapshot(), before);
        let phase = next.voices[0].phase;
        step(&mut next, &[PortFrame::default(); 4], 0);
        assert_eq!(next.voices[0].phase, phase + 2.0 * C4 / 8192.0);
        let node: Node = serde_json::from_value(json!({"id":"vco","kind":"stock_vco"})).unwrap();
        let mut other_rate = StockVco::build(&node, 48000).unwrap();
        other_rate.adopt(&old);
        assert_eq!(other_rate.voices[0].phase, 0.0);
    }
    #[test]
    fn all_widths_masks_extreme_finite_and_nonfinite_diagnostics_remain_bounded() {
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
            let mut model = model(json!({"linear_fm":linear,"fm_amount":1,"pwm_amount":1e-8}));
            for width in 0..=16 {
                for mask in 0..16 {
                    let mut inputs = [PortFrame::default(); 4];
                    inputs[0].channels = width;
                    inputs[0].values = values;
                    inputs[1] = PortFrame {
                        channels: 16,
                        values,
                    };
                    inputs[2] = PortFrame {
                        channels: 16,
                        values,
                    };
                    inputs[3] = PortFrame {
                        channels: 16,
                        values,
                    };
                    let out = step(&mut model, &inputs, mask);
                    for (i, f) in out.iter().enumerate() {
                        assert_eq!(
                            f.channels,
                            if mask & (1 << i) != 0 {
                                width.max(1)
                            } else {
                                0
                            }
                        );
                        assert!(f.values.iter().all(|v| v.is_finite()));
                    }
                    let state = model.snapshot();
                    assert!(state
                        .phases
                        .iter()
                        .all(|v| v.is_finite() && *v >= 0.0 && *v < 1.0));
                    assert!(state.filter_state.iter().flatten().all(|v| v.is_finite()));
                    assert!(state
                        .correction_tails
                        .iter()
                        .flatten()
                        .flatten()
                        .all(|v| v.is_finite()));
                    assert!(state.lights.iter().all(|v| v.is_finite()));
                }
            }
        }
    }
    #[test]
    fn strict_boolean_and_parameter_bounds_are_validated() {
        for params in [
            json!({"linear_fm":0.5}),
            json!({"soft_sync":-1}),
            json!({"frequency":76.01}),
            json!({"pulse_width":0}),
            json!({"not_a_control":1}),
        ] {
            let node =
                serde_json::from_value(json!({"id":"vco","kind":"stock_vco","params":params}))
                    .unwrap();
            assert!(StockVco::build(&node, 48000).is_err());
        }
    }
}
