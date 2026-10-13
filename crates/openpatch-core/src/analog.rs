//! OpenPatch reference building blocks. Component assumptions and limits are
//! specified in docs/ANALOG_MODELS.md; these are not commercial module clones.
use crate::ladder::Oversampler;

pub(crate) const THERMAL_VOLTAGE: f64 = 1.380649e-23 * 298.15 / 1.602176634e-19;

/// Ideal exponential current source charging a 1 nF capacitor over a 5 V ramp.
/// An ideal gain of two and a -5 V offset map the ramp to ±5 V at the jack.
/// Reset/comparator discontinuities use a two-sample polynomial BLEP correction.
/// This suppresses (but does not eliminate) aliasing; it is not a transistor VCO.
#[derive(Clone, Debug)]
pub(crate) struct Vco {
    pub(crate) phase: f64,
    rate: f64,
    frequency: f64,
    pulse: bool,
    width: f64,
}
impl Vco {
    pub(crate) fn new(rate: f64, frequency: f64, waveform: f32, width: f64) -> Self {
        Self {
            phase: 0.0,
            rate,
            frequency,
            pulse: waveform >= 0.5,
            width,
        }
    }
    pub(crate) fn process(&mut self, pitch: f32, fm: f32) -> f32 {
        let hz = (self.frequency * 2.0_f64.powf(f64::from(pitch).clamp(-16.0, 16.0))
            + f64::from(fm))
        .clamp(0.0, self.rate * 0.4);
        let dt = hz / self.rate;
        let t = self.phase;
        let value = if self.pulse {
            let raw = if t < self.width { 1.0 } else { -1.0 };
            raw + blep(t, dt) - blep((t - self.width).rem_euclid(1.0), dt)
        } else {
            2.0 * t - 1.0 - blep(t, dt)
        };
        self.phase = (self.phase + dt).fract();
        (5.0 * value) as f32
    }
}
fn blep(phase: f64, dt: f64) -> f64 {
    if dt == 0.0 {
        return 0.0;
    }
    if phase < dt {
        let x = phase / dt;
        2.0 * x - x * x - 1.0
    } else if phase > 1.0 - dt {
        let x = (phase - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// Matched differential pair, ideal voltage-controlled bias and transimpedance
/// output. a=.01, Iabc=100 µA at gain1/CV10V, R=2VT/(a*100µA).
#[derive(Clone, Debug)]
pub(crate) struct Vca {
    gain: f64,
    oversampler: Oversampler,
}
impl Vca {
    pub(crate) fn new(gain: f64) -> Self {
        Self {
            gain,
            oversampler: Oversampler::new(),
        }
    }
    pub(crate) fn process(&mut self, signal: f32, cv: f32) -> f32 {
        let gain = self.gain;
        self.oversampler
            .process_pair(f64::from(signal), f64::from(cv), |input, control| {
                let scale = 2.0 * THERMAL_VOLTAGE / 0.01;
                scale * (input / scale).tanh() * gain * (control / 10.0).clamp(0.0, 1.0)
            }) as f32
    }
    /// Independent voice lanes share FIR arithmetic while preserving each
    /// voice's chronological nonlinear evaluations and original gain.
    pub(crate) fn process_two(
        first: &mut Self,
        second: &mut Self,
        inputs: [f32; 2],
        cvs: [f32; 2],
    ) -> [f32; 2] {
        let gains = [first.gain, second.gain];
        Oversampler::process_two(
            &mut first.oversampler,
            &mut second.oversampler,
            inputs.map(f64::from),
            cvs.map(f64::from),
            |signal, control| {
                let scale = 2.0 * THERMAL_VOLTAGE / 0.01;
                [
                    scale
                        * (signal[0] / scale).tanh()
                        * gains[0]
                        * (control[0] / 10.0).clamp(0.0, 1.0),
                    scale
                        * (signal[1] / scale).tanh()
                        * gains[1]
                        * (control[1] / 10.0).clamp(0.0, 1.0),
                ]
            },
        )
        .map(|sample| sample as f32)
    }
    /// Four separate differential pairs retain their individual gain/control
    /// histories while sharing the FIR coefficient traversal.
    pub(crate) fn process_four(
        first: &mut Self,
        second: &mut Self,
        third: &mut Self,
        fourth: &mut Self,
        inputs: [f32; 4],
        cvs: [f32; 4],
    ) -> [f32; 4] {
        let gains = [first.gain, second.gain, third.gain, fourth.gain];
        Oversampler::process_four(
            &mut first.oversampler,
            &mut second.oversampler,
            &mut third.oversampler,
            &mut fourth.oversampler,
            inputs.map(f64::from),
            cvs.map(f64::from),
            |signal, control| {
                let scale = 2.0 * THERMAL_VOLTAGE / 0.01;
                std::array::from_fn(|lane| {
                    scale
                        * (signal[lane] / scale).tanh()
                        * gains[lane]
                        * (control[lane] / 10.0).clamp(0.0, 1.0)
                })
            },
        )
        .map(|sample| sample as f32)
    }
    pub(crate) fn copy_state_from(&mut self, old: &Self) {
        self.oversampler.copy_state_from(&old.oversampler);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// One 1 µF capacitor, ideal switches and comparators. Attack charges toward
/// 12 V and switches to decay at 10 V. Decay approaches 10*sustain; release 0.
/// Attack time is 0→10V; decay/release knobs specify 99% settling time.
#[derive(Clone, Debug)]
pub(crate) struct Adsr {
    value: f64,
    stage: Stage,
    gate: bool,
    retrigger: bool,
    attack_coefficient: f64,
    decay_coefficient: f64,
    release_coefficient: f64,
    sustain: f64,
}
impl Adsr {
    pub(crate) fn new(rate: f64, attack: f64, decay: f64, sustain: f64, release: f64) -> Self {
        Self {
            value: 0.0,
            stage: Stage::Idle,
            gate: false,
            retrigger: false,
            attack_coefficient: -(-6.0_f64.ln() / (attack * rate)).exp_m1(),
            decay_coefficient: -(-100.0_f64.ln() / (decay * rate)).exp_m1(),
            release_coefficient: -(-100.0_f64.ln() / (release * rate)).exp_m1(),
            sustain: 10.0 * sustain,
        }
    }
    pub(crate) fn process(&mut self, gate: f32, retrigger: f32) -> f32 {
        let gate_before = self.gate;
        let retrigger_before = self.retrigger;
        schmitt(&mut self.gate, gate);
        schmitt(&mut self.retrigger, retrigger);
        if self.gate && (!gate_before || (self.retrigger && !retrigger_before)) {
            self.stage = Stage::Attack;
        } else if !self.gate && gate_before {
            self.stage = Stage::Release;
        }
        let (target, coefficient) = match self.stage {
            Stage::Idle => (0.0, 1.0),
            Stage::Attack => (12.0, self.attack_coefficient),
            Stage::Decay => (self.sustain, self.decay_coefficient),
            Stage::Release => (0.0, self.release_coefficient),
        };
        self.value += coefficient * (target - self.value);
        if self.stage == Stage::Attack && self.value >= 10.0 - 1e-12 {
            self.value = 10.0;
            self.stage = Stage::Decay;
        } else if self.stage == Stage::Release && self.value < 1e-12 {
            self.value = 0.0;
            self.stage = Stage::Idle;
        }
        self.value as f32
    }
    pub(crate) fn copy_state_from(&mut self, old: &Self) {
        self.value = old.value;
        self.stage = old.stage;
        self.gate = old.gate;
        self.retrigger = old.retrigger;
    }
}
fn schmitt(state: &mut bool, voltage: f32) {
    if voltage >= 1.0 {
        *state = true;
    } else if voltage <= 0.1 {
        *state = false;
    }
}

/// Ideal comparator clock; gate transitions deliberately have sharp edges.
#[derive(Clone, Debug)]
pub(crate) struct Gate {
    pub(crate) phase: f64,
    increment: f64,
    duty: f64,
}
impl Gate {
    pub(crate) fn new(rate: f64, frequency: f64, duty: f64) -> Self {
        Self {
            phase: 0.0,
            increment: frequency / rate,
            duty,
        }
    }
    pub(crate) fn process(&mut self) -> f32 {
        let value = if self.phase < self.duty { 10.0 } else { 0.0 };
        self.phase = (self.phase + self.increment).fract();
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_rate_vca_modulation_preserves_signal_control_alignment() {
        let mut vca = Vca::new(1.0);
        let mut squared_error = 0.0;
        let mut squared_reference = 0.0;
        for n in 0..4800 {
            let t = n as f64 / 48000.0;
            let source = 0.01 * (std::f64::consts::TAU * 1000.0 * t).sin();
            let cv = 5.0 + 2.5 * (std::f64::consts::TAU * 500.0 * t).cos();
            let actual = vca.process(source as f32, cv as f32) as f64;
            let delayed = (n as f64 - crate::ladder::LATENCY_SAMPLES as f64) / 48000.0;
            let input = 0.01 * (std::f64::consts::TAU * 1000.0 * delayed).sin();
            let control = 0.5 + 0.25 * (std::f64::consts::TAU * 500.0 * delayed).cos();
            let scale = 2.0 * THERMAL_VOLTAGE / 0.01;
            let expected = scale * (input / scale).tanh() * control;
            if n > 256 {
                squared_error += (actual - expected).powi(2);
                squared_reference += expected.powi(2);
            }
        }
        assert!((squared_error / squared_reference).sqrt() < 0.001);
    }
    #[test]
    fn envelope_matches_rc_voltage_and_schmitt_thresholds() {
        for rate in [44100.0, 48000.0, 96000.0] {
            let mut env = Adsr::new(rate, 0.1, 0.2, 0.4, 0.3);
            let mut value = 0.0;
            for _ in 0..(rate * 0.05) as usize {
                value = env.process(10.0, 0.0);
            }
            let expected = 12.0 * (1.0 - (-6.0_f64.ln() * 0.5).exp());
            assert!((value as f64 - expected).abs() < 1e-5);
            for _ in 0..(rate * 0.05) as usize {
                value = env.process(0.5, 0.0);
            }
            assert!((value - 10.0).abs() < 1e-5, "{rate}: {value}");
            for _ in 0..(rate * 0.2) as usize {
                value = env.process(10.0, 0.0);
            }
            assert!((value - 4.06).abs() < 1e-5);
            for _ in 0..(rate * 0.3) as usize {
                value = env.process(0.0, 0.0);
            }
            assert!((value - 0.0406).abs() < 1e-5);
        }
    }
    #[test]
    fn release_retrigger_and_parameter_edits_preserve_capacitor_voltage() {
        let mut env = Adsr::new(48000.0, 0.1, 0.2, 0.4, 0.3);
        for _ in 0..1000 {
            env.process(10.0, 0.0);
        }
        let before = env.value;
        assert!((env.process(0.0, 0.0) as f64) < before);
        let mut edited = Adsr::new(48000.0, 0.5, 0.4, 0.7, 0.8);
        edited.copy_state_from(&env);
        assert_eq!(edited.value, env.value);
        let before = edited.value;
        assert!((edited.process(10.0, 0.0) as f64) > before);
        assert!(edited.value - before < 0.01);
    }
    #[test]
    fn pitch_is_one_volt_per_octave_and_blep_reduces_waveform_error() {
        for pitch in [-2.0, -1.0, 0.0, 1.0, 2.0] {
            let mut vco = Vco::new(48000.0, 100.0, 0.0, 0.5);
            let mut wraps = 0;
            for _ in 0..48000 {
                let before = vco.phase;
                assert!(vco.process(pitch, 0.0).abs() <= 5.001);
                if vco.phase < before {
                    wraps += 1;
                }
            }
            let expected = 100.0 * 2.0_f32.powf(pitch);
            assert!((wraps as f32 - expected).abs() <= 1.0);
        }
        // At an exact 6 kHz fundamental, inspect the folded fifth harmonic
        // at18kHz (also third); verify BLEP reduces residual against ideal
        // truncated additive reference, compared with the naive ramp.
        let mut vco = Vco::new(48000.0, 6000.0, 0.0, 0.5);
        let (mut corrected, mut naive) = (0.0, 0.0);
        for i in 0..4800 {
            let phase = vco.phase;
            let ideal = -(1..4)
                .map(|h| (std::f64::consts::TAU * phase * h as f64).sin() / h as f64)
                .sum::<f64>()
                * 10.0
                / std::f64::consts::PI;
            let sample = vco.process(0.0, 0.0) as f64;
            if i > 0 {
                corrected += (sample - ideal).powi(2);
                naive += (5.0 * (2.0 * phase - 1.0) - ideal).powi(2);
            }
        }
        assert!(
            corrected < naive * 0.25,
            "BLEP error {corrected}, naive {naive}"
        );
    }
}
