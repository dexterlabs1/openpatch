//! Original linear mechanical resonator: sixteen unit-length stiff-string modes.
//! Exact free propagation and zero-order-held force; this is not a calibrated
//! instrument, contact model, finite-element solver, or acoustic radiation model.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS, SIGNAL_LIMIT};
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};

pub const MODAL_RESONATOR_MODES: usize = 16;
pub(crate) const INPUTS: [&str; 3] = ["pitch", "strike", "excite"];
pub(crate) const OUTPUTS: [&str; 1] = ["out"];
const MASS: f64 = 0.5; // modal mass of a 1 m, 1 kg ideal string
const PICKUP_GAIN: f64 = 25.0; // volts per metre/second
const IMPULSE: f64 = 0.01; // newton-seconds at strength=1
const LN_1000: f64 = std::f64::consts::LN_10 * 3.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModalResonatorMode {
    /// Undamped natural frequency (Hz), not a Fourier-bin frequency.
    pub frequency_hz: f64,
    /// Frequency of the decaying free oscillation (Hz).
    pub damped_frequency_hz: f64,
    /// Time for the free amplitude envelope to fall by 60 dB (seconds).
    pub decay_seconds: f64,
    /// Signed excitation mode shape times pickup mode shape.
    pub coupling: f64,
    /// Modes above 0.45 times sample rate are omitted and their state reset.
    pub active: bool,
}

/// Validated configured modes at zero pitch CV; shared by DSP and rack display.
pub fn modal_resonator_modes(
    node: &Node,
    sample_rate: u32,
) -> Result<[ModalResonatorMode; MODAL_RESONATOR_MODES], String> {
    let controls = Controls::build(node, sample_rate)?;
    Ok(controls.modes(controls.frequency))
}

pub(crate) fn catalog() -> Value {
    json!({"kind":"modal_resonator","title":"MODAL RESONATOR","model":"physical",
        "description":"Original sixteen-mode linear mechanical resonator with stiff-string natural frequencies, positive phenomenological modal damping, force excitation and impulsive strikes. OUT is a velocity pickup. No hardware, hammer-contact, or acoustic radiation calibration claim.",
        "inputs":INPUTS,"outputs":OUTPUTS,"params":{
            "frequency":{"default":220.0,"min":20.0,"max":4000.0,"unit":"Hz","sample_rate_max_ratio":0.45,"description":"First undamped natural frequency at zero pitch CV"},
            "stiffness":{"default":0.2,"min":0.0,"max":1.0,"unit":"ratio","description":"Normalized tension/bending balance B: 0 harmonic string; 1 quadratic bending spectrum"},
            "decay":{"default":2.0,"min":0.1,"max":30.0,"unit":"s","description":"First-mode amplitude RT60"},
            "damping":{"default":0.3,"min":0.0,"max":1.0,"unit":"ratio","description":"Additional decay rate per mode index: sigma_n=sigma_1*(1+damping*(n-1))"},
            "strike_position":{"default":0.23,"min":0.01,"max":0.99,"unit":"ratio","description":"Force and impulse location along unit-length string"},
            "pickup_position":{"default":0.61,"min":0.01,"max":0.99,"unit":"ratio","description":"Velocity pickup location along unit-length string"},
            "strike_strength":{"default":1.0,"min":0.0,"max":2.0,"unit":"ratio","description":"Strike impulse = 0.01*strength N*s"},
            "level":{"default":1.0,"min":0.0,"max":2.0,"unit":"ratio"}},
        "physics":{"modes":MODAL_RESONATOR_MODES,"modal_mass_kg":MASS,"force_newtons_per_volt":1.0,"strike_impulse_newton_seconds":IMPULSE,"pickup_volts_per_metre_per_second":PICKUP_GAIN,
            "equation":"q_n'' + 2*sigma_n*q_n' + (2*pi*f_n)^2*q_n = sin(n*pi*strike_position)*F/0.5",
            "frequency":"f_n=f_1*sqrt((1-B)*n^2+B*n^4)","loss":"sigma_n=ln(1000)/decay*(1+damping*(n-1))",
            "output":"level*25*sum(sin(n*pi*pickup_position)*q_n') volts; no direct force feedthrough",
            "solver":"exact damped state transition and exact zero-order-held force, coefficients cached per voice until pitch changes",
            "mode_policy":"natural frequencies above 0.45*sample_rate are omitted and reset, never folded or clamped together",
            "modulation":"pitch is 1 V/oct; first-mode frequency clamps to 20..0.45*sample_rate. Pitch changes recompute sixteen transcendental coefficient sets per voice; audio-rate CV has higher cost and can do physical work, so static passivity does not imply passivity under modulation"},
        "trigger":{"high_voltage":1.0,"low_voltage":0.1,"policy":"Schmitt rising edge; initial high triggers; impulse added to velocity before propagation; held high does not retrigger"},
        "safety":{"input_bound_volts":SIGNAL_LIMIT,"output_bound_volts":SIGNAL_LIMIT,"nonfinite":"inputs become zero; invalid modal state resets", "state_reset_threshold":"absolute displacement or velocity >= 1e100 in SI units resets that mode; emergency overflow guard, not normal dynamics"}})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"] = json!("maximum pitch, strike, and excite input channel count, minimum one; 16 independent resonators");
    module["polyphony"]["primary_input"] = Value::Null;
    module["state_adoption"] = json!("same identity, kind and sample rate retain physical displacement, velocity and active voices across parameter edits; new coefficients apply immediately; changed incoming strike stack resets only edge detectors; sample-rate changes, initialize, retirement and activation reset state");
    for (index, unit) in ["V (1 V/oct)", "V (gate)", "V (1 N/V)"].iter().enumerate() {
        module["input_metadata"][index]["unit"] = json!(unit);
    }
}

#[derive(Clone, Copy, Debug)]
struct Controls {
    rate: f64,
    frequency: f64,
    stiffness: f64,
    decay: f64,
    damping: f64,
    source: [f64; MODAL_RESONATOR_MODES],
    pickup: [f64; MODAL_RESONATOR_MODES],
    impulse: f64,
    level: f64,
}
impl Controls {
    fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) {
            return Err("modal resonator sample rate must be 8000..=192000 Hz".into());
        }
        if node.kind != "modal_resonator" {
            return Err("expected modal_resonator node".into());
        }
        for name in node.params.keys() {
            if ![
                "frequency",
                "stiffness",
                "decay",
                "damping",
                "strike_position",
                "pickup_position",
                "strike_strength",
                "level",
            ]
            .contains(&name.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let p = |name, default, min, max| parameter(node, name, default, min, max).map(f64::from);
        let source = p("strike_position", 0.23, 0.01, 0.99)?;
        let pickup = p("pickup_position", 0.61, 0.01, 0.99)?;
        Ok(Self {
            rate: f64::from(rate),
            frequency: p("frequency", 220.0, 20.0, 4000.0_f32.min(rate as f32 * 0.45))?,
            stiffness: p("stiffness", 0.2, 0.0, 1.0)?,
            decay: p("decay", 2.0, 0.1, 30.0)?,
            damping: p("damping", 0.3, 0.0, 1.0)?,
            source: std::array::from_fn(|i| ((i + 1) as f64 * PI * source).sin()),
            pickup: std::array::from_fn(|i| ((i + 1) as f64 * PI * pickup).sin()),
            impulse: IMPULSE * p("strike_strength", 1.0, 0.0, 2.0)?,
            level: p("level", 1.0, 0.0, 2.0)?,
        })
    }
    fn modes(&self, fundamental: f64) -> [ModalResonatorMode; MODAL_RESONATOR_MODES] {
        std::array::from_fn(|i| {
            let n = (i + 1) as f64;
            let f =
                fundamental * ((1.0 - self.stiffness) * n * n + self.stiffness * n.powi(4)).sqrt();
            let sigma = LN_1000 / self.decay * (1.0 + self.damping * (n - 1.0));
            ModalResonatorMode {
                frequency_hz: f,
                damped_frequency_hz: ((TAU * f).powi(2) - sigma * sigma).sqrt() / TAU,
                decay_seconds: LN_1000 / sigma,
                coupling: self.source[i] * self.pickup[i],
                active: f <= self.rate * 0.45,
            }
        })
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Coefficient {
    a11: f64,
    a12: f64,
    a21: f64,
    a22: f64,
    force_q: f64,
    force_v: f64,
    active: bool,
}
impl Coefficient {
    fn new(f: f64, sigma: f64, h: f64) -> Self {
        let w2 = (TAU * f).powi(2);
        let d = (w2 - sigma * sigma).sqrt();
        let r = (-sigma * h).exp();
        let b = r * (d * h).sin() / d;
        let c = r * (d * h).cos();
        let a11 = c + sigma * b;
        // Avoid cancellation in 1-a11 for small h: 1-r*cos(dh)-sigma*b.
        let one_minus_a11 =
            -(-sigma * h).exp_m1() + r * 2.0 * (d * h * 0.5).sin().powi(2) - sigma * b;
        Self {
            a11,
            a12: b,
            a21: -w2 * b,
            a22: c - sigma * b,
            force_q: one_minus_a11 / w2,
            force_v: b,
            active: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct ModeState {
    q: f64,
    v: f64,
}
impl ModeState {
    fn step(&mut self, c: Coefficient, u: f64) {
        let q = c.a11 * self.q + c.a12 * self.v + c.force_q * u;
        let v = c.a21 * self.q + c.a22 * self.v + c.force_v * u;
        if q.is_finite() && v.is_finite() && q.abs() < 1e100 && v.abs() < 1e100 {
            self.q = if q.abs() < 1e-30 { 0.0 } else { q };
            self.v = if v.abs() < 1e-30 { 0.0 } else { v };
        } else {
            *self = Self::default();
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Voice {
    states: [ModeState; MODAL_RESONATOR_MODES],
    coefficients: [Coefficient; MODAL_RESONATOR_MODES],
    frequency: f64,
    high: bool,
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            states: [ModeState::default(); MODAL_RESONATOR_MODES],
            coefficients: [Coefficient::default(); MODAL_RESONATOR_MODES],
            frequency: 0.0,
            high: false,
        }
    }
}
impl Voice {
    fn process(&mut self, c: &Controls, pitch: f32, strike: f32, force: f32) -> f32 {
        let fundamental = (c.frequency * 2.0_f64.powf(f64::from(safe(pitch)).clamp(-32.0, 32.0)))
            .clamp(20.0, c.rate * 0.45);
        if fundamental != self.frequency {
            self.frequency = fundamental;
            for (i, mode) in c.modes(fundamental).iter().enumerate() {
                self.coefficients[i] = if mode.active {
                    Coefficient::new(
                        mode.frequency_hz,
                        LN_1000 / mode.decay_seconds,
                        1.0 / c.rate,
                    )
                } else {
                    self.states[i] = ModeState::default();
                    Coefficient::default()
                };
            }
        }
        let strike = safe(strike);
        let rising = !self.high && strike >= 1.0;
        if strike <= 0.1 {
            self.high = false;
        } else if rising {
            self.high = true;
        }
        let force = f64::from(safe(force));
        let mut out = 0.0;
        for i in 0..MODAL_RESONATOR_MODES {
            if !self.coefficients[i].active {
                continue;
            }
            if rising {
                self.states[i].v += c.impulse * c.source[i] / MASS;
            }
            self.states[i].step(self.coefficients[i], c.source[i] * force / MASS);
            out += c.pickup[i] * self.states[i].v;
        }
        (out * c.level * PICKUP_GAIN).clamp(-f64::from(SIGNAL_LIMIT), f64::from(SIGNAL_LIMIT))
            as f32
    }
}
fn safe(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-SIGNAL_LIMIT, SIGNAL_LIMIT)
    } else {
        0.0
    }
}
#[derive(Clone, Debug)]
pub(crate) struct ModalResonator {
    controls: Controls,
    voices: [Voice; MAX_CHANNELS],
    active: usize,
}
impl ModalResonator {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        Ok(Self {
            controls: Controls::build(node, rate)?,
            voices: [Voice::default(); MAX_CHANNELS],
            active: 0,
        })
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let count = inputs
            .iter()
            .take(3)
            .map(|f| usize::from(f.channels))
            .max()
            .unwrap_or(0)
            .clamp(1, MAX_CHANNELS);
        for i in count.min(self.active)..count.max(self.active) {
            self.voices[i] = Voice::default();
        }
        self.active = count;
        let mut out = PortFrame {
            channels: count as u8,
            ..PortFrame::default()
        };
        for (i, value) in out.values[..count].iter_mut().enumerate() {
            *value = self.voices[i].process(
                &self.controls,
                inputs[0].voice(i, 0.0),
                inputs[1].voice(i, 0.0),
                inputs[2].voice(i, 0.0),
            );
        }
        outputs[0] = out;
    }
    pub(crate) fn reset_state(&mut self) {
        self.voices.fill(Voice::default());
        self.active = 0;
    }
    pub(crate) fn adopt(&mut self, old: &Self, strike_unchanged: bool) {
        if self.controls.rate != old.controls.rate {
            return;
        }
        self.active = old.active;
        for (voice, previous) in self.voices.iter_mut().zip(&old.voices) {
            voice.states = previous.states;
            voice.high = strike_unchanged && previous.high;
            // New compiled controls remain authoritative; recache all coefficients.
            voice.frequency = 0.0;
        }
    }
}

#[cfg(test)]
mod tests;
