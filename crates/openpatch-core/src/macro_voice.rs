//! Native polyphonic macro synthesis. FM feedback and additive spectral shaping
//! are adapted from Emilie Gillet's MIT-licensed Plaits algorithms; the remaining
//! engines and host integration are original OpenPatch implementations.
//! This is a Plaits-inspired instrument, not a sample-identical Plaits port.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS};
use serde_json::{json, Value};
use std::{
    f64::consts::{SQRT_2, TAU},
    sync::OnceLock,
};

pub(crate) const INPUTS: [&str; 7] = [
    "pitch",
    "trigger",
    "harmonics",
    "timbre",
    "morph",
    "fm",
    "level",
];
pub(crate) const OUTPUTS: [&str; 2] = ["out", "aux"];
pub(crate) const ENGINES: [&str; 8] = [
    "Analog",
    "Wavefold",
    "FM",
    "Harmonic",
    "Wavetable",
    "Chord",
    "Swarm",
    "Metallic",
];
const TABLE_SIZE: usize = 4096;
const PARTIALS: usize = 24;
const OVERSAMPLE: usize = 8;
const FIR: usize = 64;

pub(crate) fn catalog() -> Value {
    json!({"kind":"macro_voice","title":"MACRO VOICE","model":"functional",
    "description":"Eight distinct native macro-synthesis engines: detuned analog, wavefolding, feedback FM, harmonic spectra, spectral wavetables, four-note chords, swarm and inharmonic metallic tones. FM/additive algorithms adapted from MIT-licensed Plaits; not a bit-identical Plaits port. Optional trigger drives an internal decaying LPG; unpatched trigger is continuous.",
    "inputs":INPUTS,"params":{
        "engine":{"default":0,"min":0,"max":7,"step":1,"values":[0,1,2,3,4,5,6,7],"labels":ENGINES},
            "frequency":{"default":261.62558,"min":8.0,"max":16000.0,"unit":"Hz","sample_rate_max_ratio":0.4},
        "harmonics":{"default":0.3,"min":0,"max":1},
        "timbre":{"default":0.45,"min":0,"max":1},
        "morph":{"default":0.4,"min":0,"max":1},
        "decay":{"default":0.5,"min":0,"max":1},
        "level":{"default":0.8,"min":0,"max":1}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"] = json!("maximum pitch/trigger width, minimum one, up to 16 independent voices; other CVs mono-broadcast and do not create voices");
    module["polyphony"]["primary_input"] = json!("pitch");
    if let Some(inputs) = module["input_metadata"].as_array_mut() {
        for input in inputs {
            if input["name"] == "level" {
                input["normalization"] = json!(10);
            }
        }
    }
    module["state_adoption"] = json!("same identity, engine and sample rate preserve phases, spectra, LPG and oversampling history; engine changes reset synthesis histories; retired voices reset; changed trigger route resets only its edge detector");
    module["signal_policy"] = json!("pitch 1 V/oct from C4; harmonics/timbre/morph add CV/5; FM is exponential 1 V/oct; level 0..10 V, normal 10 V; OUT/AUX nominal +/-5 V times LEVEL; trigger Schmitt thresholds 0.1/1 V; wavefold and FM use 8x oversampling and a 64-tap FIR");
}

struct Tables {
    sine: [f32; TABLE_SIZE + 1],
    fir: [f32; FIR],
}
fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let sine = std::array::from_fn(|i| (TAU * i as f64 / TABLE_SIZE as f64).sin() as f32);
        let mut fir = std::array::from_fn(|i| {
            let x = i as f64 - (FIR - 1) as f64 * 0.5;
            let cutoff = 0.46 / OVERSAMPLE as f64;
            let sinc = (TAU * cutoff * x).sin() / (std::f64::consts::PI * x);
            let p = i as f64 / (FIR - 1) as f64;
            let window = 0.42 - 0.5 * (TAU * p).cos() + 0.08 * (2.0 * TAU * p).cos();
            (sinc * window) as f32
        });
        let sum: f32 = fir.iter().sum();
        for coefficient in &mut fir {
            *coefficient /= sum;
        }
        Tables { sine, fir }
    })
}
fn sin(phase: f64, table: &[f32; TABLE_SIZE + 1]) -> f32 {
    let x = (phase - phase.floor()) * TABLE_SIZE as f64;
    // Tiny negative phases can round to exactly one after subtracting floor.
    // That boundary is the start of the next period, not a second guard sample.
    if x >= TABLE_SIZE as f64 {
        return table[0];
    }
    let i = x as usize;
    let f = (x - i as f64) as f32;
    table[i] + (table[i + 1] - table[i]) * f
}
fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}
fn blep(p: f64, dt: f64) -> f32 {
    if p < dt {
        let x = p / dt;
        (2.0 * x - x * x - 1.0) as f32
    } else if p > 1.0 - dt {
        let x = (p - 1.0) / dt;
        (x * x + 2.0 * x + 1.0) as f32
    } else {
        0.0
    }
}
fn saw(p: f64, dt: f64) -> f32 {
    (2.0 * p - 1.0) as f32 - blep(p, dt)
}
fn advance(phase: &mut f64, delta: f64) -> f64 {
    *phase = (*phase + delta).fract();
    *phase
}

#[derive(Clone, Copy, Debug)]
struct Controls {
    rate: f64,
    engine: usize,
    frequency: f32,
    harmonics: f32,
    timbre: f32,
    morph: f32,
    level: f32,
    smoothing: f32,
    release: f32,
}
impl Controls {
    fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if ![
                "engine",
                "frequency",
                "harmonics",
                "timbre",
                "morph",
                "decay",
                "level",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let engine = parameter(node, "engine", 0.0, 0.0, 7.0)?;
        if engine.fract() != 0.0 {
            return Err(format!("node '{}': engine must be an integer", node.id));
        }
        let decay = parameter(node, "decay", 0.5, 0.0, 1.0)?;
        let rate = f64::from(rate);
        Ok(Self {
            rate,
            engine: engine as usize,
            frequency: parameter(node, "frequency", 261.625_58, 8.0, 16000.0)?,
            harmonics: parameter(node, "harmonics", 0.3, 0.0, 1.0)?,
            timbre: parameter(node, "timbre", 0.45, 0.0, 1.0)?,
            morph: parameter(node, "morph", 0.4, 0.0, 1.0)?,
            level: parameter(node, "level", 0.8, 0.0, 1.0)?,
            smoothing: (1.0 - (-1.0 / (0.004 * rate)).exp()) as f32,
            release: (-1.0 / ((0.035 + f64::from(decay * decay) * 5.0) * rate)).exp() as f32,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    phase: [f64; 9],
    controls: [f32; 3],
    spectrum: [f32; PARTIALS],
    control_tick: u8,
    fm_previous: f32,
    fir: [[f32; FIR]; 2],
    fir_index: usize,
    gate: bool,
    envelope: f32,
    lpg: [f32; 2],
    dc_input: [f32; 2],
    dc_output: [f32; 2],
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            phase: std::array::from_fn(|i| i as f64 * 0.137),
            controls: [0.0; 3],
            spectrum: [0.0; PARTIALS],
            control_tick: 0,
            fm_previous: 0.0,
            fir: [[0.0; FIR]; 2],
            fir_index: 0,
            gate: false,
            envelope: 0.0,
            lpg: [0.0; 2],
            dc_input: [0.0; 2],
            dc_output: [0.0; 2],
        }
    }
}
impl Voice {
    fn render(
        &mut self,
        c: &Controls,
        frequency: f64,
        parameters: [f32; 3],
        table: &Tables,
    ) -> [f32; 2] {
        for (value, target) in self.controls.iter_mut().zip(parameters) {
            *value += (target - *value) * c.smoothing;
        }
        let [h, t, m] = self.controls;
        let dt = (frequency / c.rate).clamp(0.0000001, 0.45);
        match c.engine {
            0 | 6 => {
                let count = if c.engine == 6 { 7 } else { 3 };
                let spread = f64::from(h * h) * if count == 7 { 0.045 } else { 0.018 };
                let mut sum = 0.0;
                for i in 0..count {
                    let detune = (i as f64 / (count - 1) as f64 - 0.5) * spread;
                    let drift = if count == 7 {
                        sin(self.phase[7] + i as f64 * 0.17, &table.sine) as f64
                            * f64::from(m)
                            * 0.003
                    } else {
                        0.0
                    };
                    let step = (dt * (1.0 + detune + drift)).min(0.45);
                    let p = advance(&mut self.phase[i], step);
                    let a = saw(p, step);
                    let width = 0.1 + f64::from(m) * 0.8;
                    let pulse = (if p < width { 1.0 } else { -1.0 }) + blep(p, step)
                        - blep((p + 1.0 - width).fract(), step);
                    let wave = if count == 7 {
                        a * t + sin(p, &table.sine) * (1.0 - t)
                    } else {
                        a * (1.0 - t) + pulse * t
                    };
                    sum += wave / count as f32;
                }
                advance(&mut self.phase[7], 0.23 / c.rate);
                [sum, sin(advance(&mut self.phase[8], dt * 0.5), &table.sine)]
            }
            1 | 2 => {
                const RATIOS: [f64; 16] = [
                    0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0,
                    16.0,
                ];
                let ratio = RATIOS[(h * 15.0).round() as usize];
                let anti_alias =
                    ((1.0 - (frequency * ratio / 15000.0) as f32).clamp(0.0, 1.0)).powi(2);
                for _ in 0..OVERSAMPLE {
                    let step = dt / OVERSAMPLE as f64;
                    let carrier = advance(&mut self.phase[0], step);
                    let sub = advance(&mut self.phase[2], step * 0.5);
                    let pair = if c.engine == 2 {
                        // Plaits' bipolar feedback: negative modulates rate,
                        // positive feeds back phase. Index rolls off at high pitch.
                        let feedback = 2.0 * m - 1.0;
                        let phase_feedback = if feedback < 0.0 {
                            0.5 * feedback * feedback
                        } else {
                            0.0
                        };
                        let modulator = advance(
                            &mut self.phase[1],
                            (step * ratio * f64::from(1.0 + self.fm_previous * phase_feedback))
                                .clamp(0.0, 0.49),
                        );
                        let fb = if feedback > 0.0 {
                            0.25 * feedback * feedback
                        } else {
                            0.0
                        };
                        let modulator =
                            sin(modulator + f64::from(fb * self.fm_previous), &table.sine);
                        let amount = 2.0 * t * t * anti_alias;
                        let main = sin(carrier + f64::from(amount * modulator), &table.sine);
                        self.fm_previous += (main - self.fm_previous) * 0.05;
                        [
                            main,
                            sin(sub + f64::from(amount * main * 0.25), &table.sine),
                        ]
                    } else {
                        let p = sin(carrier, &table.sine);
                        let folded = sin(f64::from(p * (0.25 + 3.5 * t) + m * 0.5), &table.sine);
                        let asymmetric = (folded * (1.0 - h)
                            + sin(carrier * 2.0 + f64::from(folded * h), &table.sine) * h)
                            .tanh();
                        [asymmetric, sin(sub, &table.sine)]
                    };
                    self.fir[0][self.fir_index] = pair[0];
                    self.fir[1][self.fir_index] = pair[1];
                    self.fir_index = (self.fir_index + 1) % FIR;
                }
                std::array::from_fn(|channel| {
                    table
                        .fir
                        .iter()
                        .enumerate()
                        .map(|(i, k)| self.fir[channel][(self.fir_index + i) % FIR] * k)
                        .sum()
                })
            }
            3 | 4 => {
                if self.control_tick == 0 {
                    let slope = 0.01 + 1.99 * ((1.0 - 0.6 * h) * m).powi(3);
                    let bumps = 16.0 * h * h;
                    let margin = (1.0 / slope - 1.0) / (1.0 + bumps);
                    let center = t * (23.0 + margin) - 0.5 * margin;
                    let mut sum = 0.001;
                    for (i, amplitude) in self.spectrum.iter_mut().enumerate() {
                        let harmonic = (i + 1) as f32;
                        let gain = if c.engine == 3 {
                            // Plaits additive-engine centroid/slope/bumps shape.
                            let order = (i as f32 - center).abs() * slope;
                            let base = (1.0 - order).max(0.0) * 2.0;
                            let bump = 1.0 + sin(f64::from(0.25 + order * bumps), &table.sine);
                            (base * base * bump).powi(4)
                        } else {
                            // Continuous spectral frame scanning: organ, reed,
                            // saw and formant spectra, band-limited per voice.
                            let weights = [
                                if i == 0 { 1.0 } else { 0.0 },
                                if i % 2 == 0 { 1.0 / harmonic } else { 0.0 },
                                1.0 / harmonic,
                                (-(harmonic - (2.0 + t * 12.0)).powi(2) / (1.0 + 8.0 * h)).exp()
                                    / harmonic.sqrt(),
                            ];
                            let frame = m * 2.999;
                            let index = frame as usize;
                            weights[index] + (weights[index + 1] - weights[index]) * frame.fract()
                        };
                        let nyquist = (0.48 - harmonic as f64 * dt).clamp(0.0, 0.04) / 0.04;
                        *amplitude += ((gain * nyquist as f32) - *amplitude) * 0.05;
                        sum += *amplitude;
                    }
                    if sum > 0.0 {
                        for a in &mut self.spectrum {
                            *a /= sum;
                        }
                    }
                }
                self.control_tick = (self.control_tick + 1) % 24;
                let p = advance(&mut self.phase[0], dt);
                let mut out = 0.0;
                let mut aux = 0.0;
                for (i, a) in self.spectrum.iter().enumerate() {
                    if dt * (i + 1) as f64 >= 0.48 {
                        continue;
                    }
                    let value = sin(p * (i + 1) as f64, &table.sine) * a;
                    out += value;
                    if i % 2 == 0 {
                        aux += value;
                    }
                }
                [out, aux]
            }
            5 => {
                const CHORDS: [[f64; 4]; 8] = [
                    [0.0, 4.0, 7.0, 12.0],
                    [0.0, 3.0, 7.0, 12.0],
                    [0.0, 4.0, 7.0, 11.0],
                    [0.0, 3.0, 7.0, 10.0],
                    [0.0, 5.0, 7.0, 12.0],
                    [0.0, 4.0, 7.0, 14.0],
                    [0.0, 3.0, 7.0, 14.0],
                    [0.0, 7.0, 12.0, 19.0],
                ];
                let chord = CHORDS[(h * 7.0).round() as usize];
                let inversion = (t * 3.0).round() as usize;
                let mut out = 0.0;
                let mut root = 0.0;
                for (i, note) in chord.iter().enumerate() {
                    let note = note + if i < inversion { 12.0 } else { 0.0 };
                    let step = (dt * (note / 12.0).exp2()).min(0.45);
                    let p = advance(&mut self.phase[i], step);
                    let value = sin(p, &table.sine) * (1.0 - m) + saw(p, step) * m;
                    out += value * 0.25;
                    if i == 0 {
                        root = value;
                    }
                }
                [out, root]
            }
            _ => {
                const METAL: [f64; 6] = [1.0, SQRT_2, 1.932, 2.546, 3.37, 4.2];
                let mut out = 0.0;
                let mut aux = 0.0;
                let mut gain_sum = 0.001;
                for (i, ratio) in METAL.iter().enumerate() {
                    let ratio = (i + 1) as f64 * (1.0 - f64::from(h)) + ratio * f64::from(h);
                    let gain = (1.0 - f64::from(t) * 0.88).powi(i as i32) as f32;
                    let amplitude = if dt * ratio < 0.45 { gain } else { 0.0 };
                    let p = advance(&mut self.phase[i], (dt * ratio).min(0.45));
                    let wobble = sin(self.phase[7] + i as f64 * 0.12, &table.sine) * m * 0.025;
                    let value = sin(p + f64::from(wobble), &table.sine) * amplitude;
                    out += value;
                    if i % 2 == 0 {
                        aux += value;
                    }
                    gain_sum += amplitude;
                }
                advance(&mut self.phase[7], 0.37 / c.rate);
                [out / gain_sum, aux / gain_sum]
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct MacroVoice {
    controls: Controls,
    voices: [Voice; MAX_CHANNELS],
    active: usize,
    tables: &'static Tables,
}
impl std::fmt::Debug for Tables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MacroVoiceTables")
    }
}
impl MacroVoice {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        Ok(Self {
            controls: Controls::build(node, rate)?,
            voices: [Voice::default(); MAX_CHANNELS],
            active: 0,
            tables: tables(),
        })
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let count = usize::from(inputs[0].channels.max(inputs[1].channels)).clamp(1, MAX_CHANNELS);
        for i in count.min(self.active)..count.max(self.active) {
            self.voices[i] = Voice::default();
        }
        self.active = count;
        outputs.fill(PortFrame {
            channels: count as u8,
            ..PortFrame::default()
        });
        for i in 0..count {
            let v = &mut self.voices[i];
            let c = &self.controls;
            let trigger = finite(inputs[1].voice(i, 0.0));
            if !v.gate && trigger >= 1.0 {
                v.gate = true;
                v.envelope = 1.0;
            }
            if v.gate && trigger <= 0.1 {
                v.gate = false;
            }
            let pitch = (finite(inputs[0].voice(i, 0.0)) + finite(inputs[5].voice(i, 0.0)))
                .clamp(-16.0, 16.0);
            let frequency =
                (f64::from(c.frequency) * f64::from(pitch).exp2()).clamp(1.0, c.rate * 0.4);
            let values = std::array::from_fn(|j| {
                ([c.harmonics, c.timbre, c.morph][j] + finite(inputs[j + 2].voice(i, 0.0)) / 5.0)
                    .clamp(0.0, 1.0)
            });
            let mut pair = v.render(c, frequency, values, self.tables);
            let amplitude = if inputs[1].channels > 0 {
                v.envelope *= c.release;
                v.envelope
            } else {
                1.0
            };
            let level = finite(inputs[6].voice(i, 10.0)).clamp(0.0, 10.0) * 0.1 * c.level;
            let cutoff = if inputs[1].channels > 0 {
                200.0 + v.envelope * v.envelope * (4000.0 + 12000.0 * c.morph)
            } else {
                (c.rate * 0.45) as f32
            };
            let coefficient = (1.0 - (-TAU * f64::from(cutoff) / c.rate).exp()) as f32;
            let dc = (-TAU * 5.0 / c.rate).exp() as f32;
            for channel in 0..2 {
                v.lpg[channel] += (pair[channel] - v.lpg[channel]) * coefficient;
                pair[channel] = v.lpg[channel] * amplitude * level * 5.0;
                let value = pair[channel] - v.dc_input[channel] + dc * v.dc_output[channel];
                v.dc_input[channel] = pair[channel];
                v.dc_output[channel] = value;
                outputs[channel].values[i] = finite(value).clamp(-10.0, 10.0);
            }
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.voices.fill(Voice::default());
        self.active = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self, trigger_unchanged: bool) {
        if self.controls.rate == old.controls.rate && self.controls.engine == old.controls.engine {
            self.voices = old.voices;
            self.active = old.active;
            if !trigger_unchanged {
                for v in &mut self.voices {
                    v.gate = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
