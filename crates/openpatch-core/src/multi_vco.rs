//! Dedicated version-2 functional VCO. The four jacks share one per-voice
//! phase timeline. This module is original Rust code, not a circuit model.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS};
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};
use std::sync::OnceLock;

pub(crate) const INPUTS: [&str; 4] = ["pitch", "fm", "sync", "pwm"];
pub(crate) const OUTPUTS: [&str; 4] = ["sine", "triangle", "saw", "square"];
const C4_HZ: f32 = 261.625_58;
const MIN_KNOB_HZ: f32 = 11.562_326;
const MAX_KNOB_HZ: f32 = 5_919.911;

pub(crate) fn catalog() -> Value {
    json!({
        "kind": "multi_vco",
        "title": "MULTI VCO",
        "model": "functional",
        "description": "Four phase-coherent audio-rate outputs with pitch, FM, sync, and pulse-width modulation. Original functional DSP; no physical-circuit claim.",
        "inputs": INPUTS,
        "params": {
            "frequency": {"default": C4_HZ, "min": MIN_KNOB_HZ, "max": MAX_KNOB_HZ, "unit": "Hz", "description": "C4 default; knob range is ±4.5 octaves"},
            "fm_amount": {"default": 0.0, "min": -1.0, "max": 1.0},
            "pulse_width": {"default": 0.5, "min": 0.01, "max": 0.99, "unit": "ratio"},
            "pwm_amount": {"default": 0.0, "min": -1.0, "max": 1.0},
            "linear_fm": {"default": 0.0, "min": 0.0, "max": 1.0, "step": 1.0},
            "soft_sync": {"default": 0.0, "min": 0.0, "max": 1.0, "step": 1.0}
        }
    })
}

pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"] = json!("pitch input sets 1–16 voices, minimum one; auxiliary inputs mono-broadcast or use zero for absent short-poly lanes");
    module["polyphony"]["primary_input"] = json!("pitch");
    module["state_adoption"] = json!("same ID, kind, and sample rate retain phase, direction, sync history, correction tails, and square filter; changed incoming sync stack resets only the sync detector; retired voices reset");
    module["compatibility"] = json!("original functional implementation targeting Fundamental 2.6.4 VCO behavior; legacy analog_vco remains independent");
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Controls {
    pub(crate) rate: f64,
    pub(crate) frequency: f64,
    pub(crate) fm_amount: f64,
    pub(crate) pulse_width: f64,
    pub(crate) pwm_amount: f64,
    pub(crate) linear_fm: bool,
    pub(crate) soft_sync: bool,
    pub(crate) highpass_feedback: f64,
    pub(crate) highpass_input: f64,
}
impl Controls {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if ![
                "frequency",
                "fm_amount",
                "pulse_width",
                "pwm_amount",
                "linear_fm",
                "soft_sync",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let flag = |name| -> Result<bool, String> {
            let value = parameter(node, name, 0.0, 0.0, 1.0)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value == 1.0)
        };
        // Bilinear RC high-pass: the input difference and old output have
        // distinct coefficients. This keeps the 20 Hz corner well behaved.
        let hp_ratio = PI * 20.0 / f64::from(rate);
        Ok(Self {
            rate: f64::from(rate),
            frequency: f64::from(parameter(
                node,
                "frequency",
                C4_HZ,
                MIN_KNOB_HZ,
                MAX_KNOB_HZ,
            )?),
            fm_amount: f64::from(parameter(node, "fm_amount", 0.0, -1.0, 1.0)?),
            pulse_width: f64::from(parameter(node, "pulse_width", 0.5, 0.01, 0.99)?),
            pwm_amount: f64::from(parameter(node, "pwm_amount", 0.0, -1.0, 1.0)?),
            linear_fm: flag("linear_fm")?,
            soft_sync: flag("soft_sync")?,
            highpass_feedback: (1.0 - hp_ratio) / (1.0 + hp_ratio),
            highpass_input: 1.0 / (1.0 + hp_ratio),
        })
    }
}

const OVERSAMPLE: usize = 16;
const TAIL_SAMPLES: usize = 32;
const IMPULSE_SAMPLES: usize = OVERSAMPLE * TAIL_SAMPLES;
const FFT_SAMPLES: usize = IMPULSE_SAMPLES;
static KERNEL: OnceLock<CorrectionKernel> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default)]
struct Complex {
    re: f64,
    im: f64,
}
impl Complex {
    fn magnitude(self) -> f64 {
        self.re.hypot(self.im)
    }
}

// Control-thread coefficient generation. This radix-2 FFT and cepstral
// spectral factorization are independently authored for OpenPatch.
fn fft(values: &mut [Complex; FFT_SAMPLES], inverse: bool) {
    let mut j = 0;
    for i in 1..FFT_SAMPLES {
        let mut bit = FFT_SAMPLES >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            values.swap(i, j);
        }
    }
    let mut length = 2;
    while length <= FFT_SAMPLES {
        let angle = (if inverse { TAU } else { -TAU }) / length as f64;
        let rotation = Complex {
            re: angle.cos(),
            im: angle.sin(),
        };
        for start in (0..FFT_SAMPLES).step_by(length) {
            let mut twiddle = Complex { re: 1.0, im: 0.0 };
            for offset in 0..length / 2 {
                let a = values[start + offset];
                let b = values[start + offset + length / 2];
                let rotated = Complex {
                    re: b.re * twiddle.re - b.im * twiddle.im,
                    im: b.re * twiddle.im + b.im * twiddle.re,
                };
                values[start + offset] = Complex {
                    re: a.re + rotated.re,
                    im: a.im + rotated.im,
                };
                values[start + offset + length / 2] = Complex {
                    re: a.re - rotated.re,
                    im: a.im - rotated.im,
                };
                twiddle = Complex {
                    re: twiddle.re * rotation.re - twiddle.im * rotation.im,
                    im: twiddle.re * rotation.im + twiddle.im * rotation.re,
                };
            }
        }
        length <<= 1;
    }
    if inverse {
        for value in values.iter_mut() {
            value.re /= FFT_SAMPLES as f64;
            value.im /= FFT_SAMPLES as f64;
        }
    }
}

#[derive(Debug)]
struct CorrectionKernel {
    step: [f64; IMPULSE_SAMPLES + 1],
}
impl CorrectionKernel {
    fn generate() -> Self {
        let mut spectrum = [Complex::default(); FFT_SAMPLES];
        for (index, value) in spectrum[..IMPULSE_SAMPLES].iter_mut().enumerate() {
            // Sample the 32-host-sample sinc support with both endpoints.
            // The four-term window suppresses its truncated stopband before
            // the independently implemented cepstral minimum-phase transform.
            let progress = index as f64 / (IMPULSE_SAMPLES - 1) as f64;
            let x = TAIL_SAMPLES as f64 * (progress - 0.5);
            let sinc = if x == 0.0 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let window = 0.358_75 - 0.488_29 * (TAU * progress).cos()
                + 0.141_28 * (2.0 * TAU * progress).cos()
                - 0.011_68 * (3.0 * TAU * progress).cos();
            value.re = sinc * window;
        }
        fft(&mut spectrum, false);
        for value in &mut spectrum {
            value.re = value.magnitude().ln().max(-30.0);
            value.im = 0.0;
        }
        fft(&mut spectrum, true);
        for value in &mut spectrum[1..FFT_SAMPLES / 2] {
            value.re *= 2.0;
            value.im = 0.0;
        }
        spectrum[FFT_SAMPLES / 2] = Complex::default();
        spectrum[FFT_SAMPLES / 2 + 1..].fill(Complex::default());
        fft(&mut spectrum, false);
        for value in &mut spectrum {
            let amplitude = value.re.exp();
            let angle = value.im;
            *value = Complex {
                re: amplitude * angle.cos(),
                im: amplitude * angle.sin(),
            };
        }
        fft(&mut spectrum, true);
        let retained: f64 = spectrum[..IMPULSE_SAMPLES].iter().map(|v| v.re).sum();
        assert!(retained.is_finite() && retained.abs() > 1e-9);
        let mut step = [0.0; IMPULSE_SAMPLES + 1];
        let mut running = 0.0;
        for (index, value) in spectrum[..IMPULSE_SAMPLES].iter().enumerate() {
            running += value.re;
            step[index] = running / retained;
        }
        step[IMPULSE_SAMPLES] = 1.0;
        assert!(step.iter().all(|value| value.is_finite()));
        Self { step }
    }

    fn residual(&self, age: f64) -> f64 {
        if age >= TAIL_SAMPLES as f64 {
            return 0.0;
        }
        let position = age.max(0.0) * OVERSAMPLE as f64;
        let index = position.floor() as usize;
        let fraction = position - index as f64;
        let first = self.step[index];
        let next = self.step[index + 1];
        first + fraction * (next - first) - 1.0
    }
}

fn finite(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value)
    } else {
        0.0
    }
}

// The curved Fundamental-style waveforms are evaluated from a common phase.
// Reconstruction below corrects value jumps only; derivative discontinuities
// remain a measured aliasing gap rather than a claimed bandlimited result.
fn curve(x: f64) -> f64 {
    (5.0 * x * x - 13.0 * x + 3.0) / (2.0 * x + 3.0)
}

fn waves(phase: f64, square_high: bool) -> [f64; 4] {
    let sine = if phase < 0.5 {
        1.0 - 16.0 * (phase - 0.25).powi(2)
    } else {
        -1.0 + 16.0 * (phase - 0.75).powi(2)
    };
    let x = (phase + 0.25).fract();
    let triangle = curve((2.0 * x).fract()) * if x < 0.5 { -1.0 } else { 1.0 };
    let saw = -curve((phase + 0.5).fract());
    let square = if square_high { 1.0 } else { -1.0 };
    [sine, triangle, saw, square]
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    phase: f64,
    direction: f64,
    previous_sync: f64,
    square_high: bool,
    correction: [[f64; TAIL_SAMPLES]; 4],
    cursor: usize,
    square_previous: f64,
    square_highpass: f64,
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            phase: 0.0,
            direction: 1.0,
            previous_sync: 0.0,
            square_high: true,
            correction: [[0.0; TAIL_SAMPLES]; 4],
            cursor: 0,
            square_previous: 0.0,
            square_highpass: 0.0,
        }
    }
}
impl Voice {
    fn schedule(&mut self, output: usize, jump: f64, tau: f64, kernel: &CorrectionKernel) {
        if jump == 0.0 {
            return;
        }
        for offset in 0..TAIL_SAMPLES {
            // Output is evaluated at the end of this sample's trajectory.
            let age = 1.0 - tau + offset as f64;
            let slot = (self.cursor + offset) % TAIL_SAMPLES;
            self.correction[output][slot] += jump * kernel.residual(age);
        }
    }

    fn set_square_high(&mut self, high: bool, tau: f64, kernel: &CorrectionKernel) {
        if self.square_high == high {
            return;
        }
        // Include events exactly at the sample boundary: their first
        // correction is one host sample old at this frame's output.
        if (0.0..=1.0).contains(&tau) {
            self.schedule(3, if high { 2.0 } else { -2.0 }, tau, kernel);
        }
        self.square_high = high;
    }

    fn advance(&mut self, increment: f64, width: f64, kernel: &CorrectionKernel) {
        let forward = if increment == 0.0 {
            self.direction > 0.0
        } else {
            increment > 0.0
        };
        let past_pulse = if forward {
            self.phase >= width
        } else {
            self.phase < width
        };
        if self.square_high == forward && past_pulse {
            self.set_square_high(!forward, 0.0, kernel);
        }
        if increment == 0.0 {
            return;
        }
        let start_phase = self.phase;
        let unwrapped = start_phase + increment;
        self.phase = unwrapped.rem_euclid(1.0);
        let direction = if forward { 1.0 } else { -1.0 };
        let wrap_tau = if (forward && unwrapped >= 1.0) || (!forward && unwrapped <= 0.0) {
            let boundary = if forward { 1.0 } else { 0.0 };
            Some((boundary - start_phase) / increment)
        } else {
            None
        };
        // At most one periodic pulse edge and one wrap fit a .35-cycle step.
        // Sorting their fractions preserves narrow pulses on either side of
        // the wrap, even when the end phase lies past neither edge.
        let mut pulse_tau = None;
        for cycle in -1..=1 {
            let tau = (width + f64::from(cycle) - start_phase) / increment;
            if (0.0..=1.0).contains(&tau) {
                pulse_tau = Some(tau);
                break;
            }
        }
        if let (Some(pulse), Some(wrap)) = (pulse_tau, wrap_tau) {
            if pulse < wrap {
                if self.square_high == forward {
                    self.set_square_high(!forward, pulse, kernel);
                }
                self.set_square_high(forward, wrap, kernel);
            } else {
                self.set_square_high(forward, wrap, kernel);
                if self.square_high == forward {
                    self.set_square_high(!forward, pulse, kernel);
                }
            }
        } else if let Some(pulse) = pulse_tau {
            if self.square_high == forward {
                self.set_square_high(!forward, pulse, kernel);
            }
        } else if let Some(wrap) = wrap_tau {
            self.set_square_high(forward, wrap, kernel);
        }
        let half_tau = (0.5 - (self.phase - increment)) / increment;
        if half_tau > 0.0 && half_tau <= 1.0 {
            self.schedule(2, -2.0 * direction, half_tau, kernel);
        }
    }

    fn sample(&mut self, highpass_feedback: f64, highpass_input: f64) -> [f32; 4] {
        let mut values = waves(self.phase, self.square_high);
        for (output, value) in values.iter_mut().enumerate() {
            *value += self.correction[output][self.cursor];
            self.correction[output][self.cursor] = 0.0;
        }
        self.square_highpass = highpass_feedback * self.square_highpass
            + highpass_input * (values[3] - self.square_previous);
        self.square_previous = values[3];
        values[3] = self.square_highpass * 0.95;
        values.map(|value| (5.0 * value) as f32)
    }
}

#[derive(Debug)]
pub(crate) struct MultiVco {
    controls: Controls,
    kernel: &'static CorrectionKernel,
    active: u8,
    voices: [Voice; MAX_CHANNELS],
}
impl MultiVco {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        let controls = Controls::build(node, rate)?;
        // All FFT work and allocation complete before the first callback.
        let kernel = KERNEL.get_or_init(CorrectionKernel::generate);
        Ok(Self {
            controls,
            kernel,
            active: 0,
            voices: [Voice::default(); MAX_CHANNELS],
        })
    }

    pub(crate) fn adopt(&mut self, old: &Self, sync_unchanged: bool) {
        self.voices = old.voices;
        self.active = old.active;
        if !sync_unchanged {
            for voice in &mut self.voices {
                voice.previous_sync = 0.0;
            }
        }
    }

    pub(crate) fn process(&mut self, inputs: &[PortFrame; 4], outputs: &mut [PortFrame]) {
        let count = inputs[0].channels.max(1);
        for voice in &mut self.voices
            [usize::from(self.active.min(count))..usize::from(self.active.max(count))]
        {
            *voice = Voice::default();
        }
        self.active = count;
        outputs.fill(PortFrame {
            channels: count,
            ..PortFrame::default()
        });
        let controls = self.controls;
        let highpass_feedback = controls.highpass_feedback;
        let highpass_input = controls.highpass_input;
        for (index, voice) in self.voices[..usize::from(count)].iter_mut().enumerate() {
            let pitch = finite(inputs[0].voice(index, 0.0));
            let fm = finite(inputs[1].voice(index, 0.0));
            let sync = finite(inputs[2].voice(index, 0.0));
            let pwm = finite(inputs[3].voice(index, 0.0));
            let width = (controls.pulse_width + controls.pwm_amount * pwm / 10.0).clamp(0.01, 0.99);
            let base = controls.frequency * pitch.clamp(-64.0, 64.0).exp2();
            let hz = if controls.linear_fm {
                base + controls.fm_amount * fm * f64::from(C4_HZ)
            } else {
                controls.frequency * (pitch + controls.fm_amount * fm).clamp(-64.0, 64.0).exp2()
            }
            .clamp(0.0, controls.rate * 0.35);
            // A disconnected sync jack does not feed zero to the edge
            // detector; it leaves its last connected sample in history.
            let crossing = if inputs[2].channels == 0 {
                None
            } else {
                let difference = sync - voice.previous_sync;
                let tau = -voice.previous_sync / difference;
                // A sample ending at exactly zero counts even when it came
                // from positive voltage; the finite fraction and endpoint
                // guard define the reference's sync event.
                let crossing =
                    (sync >= 0.0 && tau.is_finite() && tau > 0.0 && tau <= 1.0).then_some(tau);
                voice.previous_sync = sync;
                crossing
            };
            if !controls.soft_sync {
                voice.direction = 1.0;
            }
            let increment = voice.direction * hz / controls.rate;
            // The host oscillator first evaluates a complete phase interval,
            // then applies sync to its end state. This ordering matters when
            // a PWM or saw edge and hard sync occur in the same sample.
            voice.advance(increment, width, self.kernel);
            if let Some(tau) = crossing.filter(|_| !controls.soft_sync) {
                let before = waves(voice.phase, voice.square_high);
                let new_phase = (1.0 - tau) * increment;
                let after = waves(new_phase, true);
                for output in 0..4 {
                    voice.schedule(output, after[output] - before[output], tau, self.kernel);
                }
                voice.phase = new_phase;
                voice.square_high = true;
            } else if crossing.is_some() && controls.soft_sync {
                voice.direction = -voice.direction;
            }
            let values = voice.sample(highpass_feedback, highpass_input);
            for (output, value) in outputs.iter_mut().zip(values) {
                output.values[index] = value;
            }
            voice.cursor = (voice.cursor + 1) % TAIL_SAMPLES;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    fn vco(params: &[(&str, f32)]) -> MultiVco {
        MultiVco::build(
            &Node {
                content: None,
                score: None,
                id: "vco".into(),
                kind: "multi_vco".into(),
                bypassed: false,
                params: params
                    .iter()
                    .map(|(key, value)| (key.to_string(), *value))
                    .collect::<BTreeMap<_, _>>(),
            },
            48_000,
        )
        .unwrap()
    }

    #[test]
    fn minimum_phase_kernel_is_finite_normalized_and_reaches_zero_residual() {
        let kernel = CorrectionKernel::generate();
        assert_eq!((OVERSAMPLE, IMPULSE_SAMPLES, FFT_SAMPLES), (16, 512, 512));
        assert!(kernel.step.iter().all(|step| step.is_finite()));
        assert!((kernel.step[IMPULSE_SAMPLES - 1] - 1.0).abs() < 1e-12);
        assert_eq!(kernel.step[IMPULSE_SAMPLES], 1.0);
        assert_eq!(
            kernel.residual(IMPULSE_SAMPLES as f64 / OVERSAMPLE as f64),
            0.0
        );
        assert_eq!(kernel.residual(TAIL_SAMPLES as f64), 0.0);
        assert!(kernel.residual(0.0).abs() > 0.01);
    }

    #[test]
    fn periodic_pwm_boundaries_after_forward_and_reverse_wrap_are_corrected() {
        let kernel = CorrectionKernel::generate();
        let mut forward = Voice {
            phase: 0.99,
            square_high: false,
            ..Voice::default()
        };
        let mut expected = forward;
        expected.schedule(3, 2.0, 1.0 / 3.0, &kernel);
        expected.schedule(3, -2.0, 2.0 / 3.0, &kernel);
        forward.advance(0.03, 0.01, &kernel);
        for (actual, expected) in forward.correction[3].iter().zip(expected.correction[3]) {
            assert!((actual - expected).abs() < 1e-11);
        }
        assert!((forward.phase - 0.02).abs() < 1e-12);
        assert!(!forward.square_high);

        let mut reverse = Voice {
            phase: 0.01,
            direction: -1.0,
            ..Voice::default()
        };
        let mut expected = reverse;
        expected.schedule(3, -2.0, 0.1, &kernel);
        expected.schedule(3, 2.0, 0.2, &kernel);
        reverse.advance(-0.1, 0.99, &kernel);
        for (actual, expected) in reverse.correction[3].iter().zip(expected.correction[3]) {
            assert!((actual - expected).abs() < 1e-11);
        }
        assert!((reverse.phase - 0.91).abs() < 1e-12);
        assert!(reverse.square_high);
    }

    #[test]
    fn narrow_pulse_before_wrap_gets_both_actual_jumps_in_time_order() {
        let kernel = CorrectionKernel::generate();
        let mut forward = Voice {
            phase: 0.98,
            square_high: true,
            ..Voice::default()
        };
        let mut expected = forward;
        expected.schedule(3, -2.0, 1.0 / 3.0, &kernel);
        expected.schedule(3, 2.0, 2.0 / 3.0, &kernel);
        forward.advance(0.03, 0.99, &kernel);
        assert!(forward.square_high);
        assert!((forward.phase - 0.01).abs() < 1e-12);
        for (actual, expected) in forward.correction[3].iter().zip(expected.correction[3]) {
            assert!((actual - expected).abs() < 1e-11);
        }

        let mut reverse = Voice {
            phase: 0.02,
            direction: -1.0,
            square_high: false,
            ..Voice::default()
        };
        let mut expected = reverse;
        expected.schedule(3, 2.0, 1.0 / 3.0, &kernel);
        expected.schedule(3, -2.0, 2.0 / 3.0, &kernel);
        reverse.advance(-0.03, 0.01, &kernel);
        assert!(!reverse.square_high);
        assert!((reverse.phase - 0.99).abs() < 1e-12);
        for (actual, expected) in reverse.correction[3].iter().zip(expected.correction[3]) {
            assert!((actual - expected).abs() < 1e-11);
        }
    }

    #[test]
    fn hard_mode_restores_forward_direction_without_a_sync_edge() {
        let mut vco = vco(&[("soft_sync", 1.0)]);
        vco.voices[0].phase = 0.2;
        vco.voices[0].direction = -1.0;
        vco.active = 1;
        vco.controls.soft_sync = false;
        let mut outputs = [PortFrame::default(); 4];
        vco.process(&[PortFrame::default(); 4], &mut outputs);
        assert_eq!(vco.voices[0].direction, 1.0);
        assert!(vco.voices[0].phase > 0.2);
    }

    #[test]
    fn soft_sync_reverses_the_following_interval() {
        let mut vco = vco(&[("soft_sync", 1.0), ("frequency", 480.0)]);
        vco.active = 1;
        vco.voices[0].phase = 0.4;
        vco.voices[0].previous_sync = -1.0;
        let mut inputs = [PortFrame::default(); 4];
        let mut outputs = [PortFrame::default(); 4];
        inputs[2] = PortFrame::mono(1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.41).abs() < 1e-10);
        assert_eq!(vco.voices[0].direction, -1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.4).abs() < 1e-10);
    }

    #[test]
    fn pwm_can_close_a_latch_but_cannot_reopen_it_before_wrap() {
        let kernel = CorrectionKernel::generate();
        let mut voice = Voice {
            phase: 0.2,
            ..Voice::default()
        };
        voice.advance(0.01, 0.1, &kernel);
        assert!(!voice.square_high);
        assert!((voice.correction[3][0] + 2.0 * kernel.residual(1.0)).abs() < 1e-12);
        let after_close = voice.correction[3];
        voice.advance(0.01, 0.3, &kernel);
        assert!(!voice.square_high);
        assert_eq!(voice.correction[3], after_close);
        voice.phase = 0.99;
        voice.advance(0.03, 0.3, &kernel);
        assert!(voice.square_high);
    }

    #[test]
    fn parked_pwm_edge_and_reverse_wrap_at_zero_have_correction_tails() {
        let kernel = CorrectionKernel::generate();
        let mut parked = Voice {
            phase: 0.2,
            ..Voice::default()
        };
        parked.advance(0.0, 0.1, &kernel);
        assert!(!parked.square_high);
        assert!((parked.correction[3][0] + 2.0 * kernel.residual(1.0)).abs() < 1e-12);

        let mut reverse = Voice {
            phase: 0.0,
            direction: -1.0,
            ..Voice::default()
        };
        reverse.advance(-0.01, 0.5, &kernel);
        assert!(!reverse.square_high);
        assert!((reverse.phase - 0.99).abs() < 1e-12);
        assert!((reverse.correction[3][0] + 2.0 * kernel.residual(1.0)).abs() < 1e-12);
    }

    #[test]
    fn hard_sync_uses_complete_interval_before_reset() {
        let mut vco = vco(&[("frequency", 1_440.0)]);
        vco.active = 1;
        vco.voices[0].phase = 0.49;
        vco.voices[0].previous_sync = -1.0;
        let mut inputs = [PortFrame::default(); 4];
        inputs[2] = PortFrame::mono(1.0);
        let mut outputs = [PortFrame::default(); 4];
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.015).abs() < 1e-12);
        // The full step passed the saw seam before the sync edge was applied.
        assert!(vco.voices[0].correction[2].iter().any(|x| x.abs() > 0.01));
    }

    #[test]
    fn sync_disconnection_holds_detector_history() {
        let mut vco = vco(&[("frequency", 480.0)]);
        vco.active = 1;
        vco.voices[0].phase = 0.4;
        let mut inputs = [PortFrame::default(); 4];
        let mut outputs = [PortFrame::default(); 4];
        inputs[2] = PortFrame::mono(1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.41).abs() < 1e-10);
        inputs[2] = PortFrame::mono(-1.0);
        vco.process(&inputs, &mut outputs);
        assert_eq!(vco.voices[0].previous_sync, -1.0);
        inputs[2] = PortFrame::default();
        vco.process(&inputs, &mut outputs);
        assert_eq!(vco.voices[0].previous_sync, -1.0);
        inputs[2] = PortFrame::mono(1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.005).abs() < 1e-10);
    }

    #[test]
    fn sync_finite_crossing_keeps_hard_reset_remainder() {
        let mut vco = vco(&[("frequency", 480.0)]);
        let mut inputs = [PortFrame::default(); 4];
        let mut outputs = [PortFrame::default(); 4];
        vco.voices[0].phase = 0.4;
        vco.active = 1;
        inputs[2] = PortFrame::mono(1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.41).abs() < 1e-10);
        inputs[2] = PortFrame::mono(-1.0);
        vco.process(&inputs, &mut outputs);
        inputs[2] = PortFrame::mono(1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.005).abs() < 1e-10);
    }

    #[test]
    fn positive_to_exact_zero_syncs_but_positive_to_negative_does_not() {
        let mut vco = vco(&[("frequency", 480.0)]);
        vco.active = 1;
        vco.voices[0].phase = 0.4;
        let mut inputs = [PortFrame::default(); 4];
        let mut outputs = [PortFrame::default(); 4];
        inputs[2] = PortFrame::mono(1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.41).abs() < 1e-10);
        inputs[2] = PortFrame::mono(0.0);
        vco.process(&inputs, &mut outputs);
        assert!(vco.voices[0].phase.abs() < 1e-10);

        vco.voices[0].phase = 0.4;
        vco.voices[0].previous_sync = 1.0;
        inputs[2] = PortFrame::mono(-1.0);
        vco.process(&inputs, &mut outputs);
        assert!((vco.voices[0].phase - 0.41).abs() < 1e-10);
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ReferenceCase {
        rate: u32,
        frames: usize,
        events: Vec<ReferenceEvent>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ReferenceEvent {
        frame: usize,
        #[serde(default)]
        params: BTreeMap<String, f32>,
        #[serde(default)]
        inputs: BTreeMap<String, Vec<f32>>,
    }

    /// Test-only numeric export for an independently built pinned reference.
    /// Run explicitly with OPENPATCH_VCO_CASE and OPENPATCH_VCO_OUTPUT paths.
    #[test]
    #[ignore]
    fn export_reference_case() {
        let Ok(case_path) = std::env::var("OPENPATCH_VCO_CASE") else {
            return;
        };
        let output_path = std::env::var("OPENPATCH_VCO_OUTPUT").expect("output path required");
        let case: ReferenceCase =
            serde_json::from_slice(&std::fs::read(case_path).unwrap()).unwrap();
        assert!((8000..=192_000).contains(&case.rate));
        let mut params = BTreeMap::<String, f32>::new();
        let make_node = |params: BTreeMap<String, f32>| Node {
            content: None,
            score: None,
            id: "vco".into(),
            kind: "multi_vco".into(),
            bypassed: false,
            params,
        };
        let mut vco = MultiVco::build(&make_node(params.clone()), case.rate).unwrap();
        let mut inputs = [PortFrame::default(); 4];
        let mut outputs = [PortFrame::default(); 4];
        let mut events = case.events;
        events.sort_by_key(|event| event.frame);
        assert!(events.iter().all(|event| event.frame < case.frames));
        let mut next_event = 0;
        let mut bytes = Vec::with_capacity(case.frames * MAX_CHANNELS * OUTPUTS.len() * 4);
        for frame in 0..case.frames {
            let mut changed_params = false;
            while events
                .get(next_event)
                .is_some_and(|event| event.frame == frame)
            {
                let event = &events[next_event];
                for (name, value) in &event.params {
                    params.insert(name.clone(), *value);
                    changed_params = true;
                }
                for (name, values) in &event.inputs {
                    let port = INPUTS
                        .iter()
                        .position(|port| *port == name.as_str())
                        .expect("unknown input");
                    assert!(values.len() <= MAX_CHANNELS);
                    let mut input = PortFrame {
                        channels: values.len() as u8,
                        ..PortFrame::default()
                    };
                    input.values[..values.len()].copy_from_slice(values);
                    inputs[port] = input;
                }
                next_event += 1;
            }
            if changed_params {
                let mut replacement =
                    MultiVco::build(&make_node(params.clone()), case.rate).unwrap();
                replacement.adopt(&vco, true);
                vco = replacement;
            }
            vco.process(&inputs, &mut outputs);
            for voice in 0..MAX_CHANNELS {
                for output in &outputs {
                    let sample = if voice < usize::from(output.channels) {
                        output.values[voice]
                    } else {
                        0.0
                    };
                    bytes.extend_from_slice(&sample.to_le_bytes());
                }
            }
        }
        std::fs::write(output_path, bytes).unwrap();
    }
}
