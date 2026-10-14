//! Stereo output conversion with optional original 10 Hz DC rejection.
//! The bypass path preserves the existing f32 unity-sum / 10 * gain order.
use crate::{parameter, Node, PortFrame};
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub(crate) struct StereoOutput {
    rate: u32,
    gain: f32,
    dc_block: bool,
    feed: f64,
    feedback: f64,
    previous_input: [f64; 2],
    previous_output: [f64; 2],
    prime: bool,
}
impl StereoOutput {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(100..=384000).contains(&rate) {
            return Err("stereo output sample rate must be in 100..=384000 Hz".into());
        }
        for name in node.params.keys() {
            if !matches!(name.as_str(), "gain" | "dc_block") {
                return Err(format!("unknown stereo output parameter {name}"));
            }
        }
        let gain = parameter(node, "gain", 1.0, 0.0, 2.0)?;
        let dc = parameter(node, "dc_block", 0.0, 0.0, 1.0)?;
        if dc.fract() != 0.0 {
            return Err("dc_block must be 0 or 1".into());
        }
        // Bilinear transform, prewarped to a digital -3 dB corner at 10 Hz.
        let corner = (PI * 10.0 / f64::from(rate)).tan();
        Ok(Self {
            rate,
            gain,
            dc_block: dc == 1.0,
            feed: 1.0 / (1.0 + corner),
            feedback: (1.0 - corner) / (1.0 + corner),
            previous_input: [0.0; 2],
            previous_output: [0.0; 2],
            prime: false,
        })
    }
    pub(crate) fn process(&mut self, left: PortFrame, right: PortFrame) -> PortFrame {
        let sum = |frame: PortFrame| {
            let value = frame.values[..usize::from(frame.channels).min(frame.values.len())]
                .iter()
                .sum::<f32>()
                / 10.0
                * self.gain;
            if value.is_finite() {
                value
            } else {
                0.0
            }
        };
        let left = sum(left);
        let right = if right.channels == 0 {
            left
        } else {
            sum(right)
        };
        let mut frame = PortFrame {
            channels: 2,
            ..PortFrame::default()
        };
        frame.values[0] = left;
        frame.values[1] = right;
        if self.dc_block {
            for (i, value) in frame.values[..2].iter_mut().enumerate() {
                let input = f64::from(*value);
                if self.prime {
                    self.previous_input[i] = input;
                    self.previous_output[i] = 0.0;
                }
                let output = self.feed * (input - self.previous_input[i])
                    + self.feedback * self.previous_output[i];
                self.previous_input[i] = input;
                self.previous_output[i] = output;
                // Engine-generated inputs are bounded well below f32::MAX.
                // Keep hostile direct callers from overflowing the conversion.
                *value = output.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32;
            }
            self.prime = false;
        }
        frame
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate == old.rate && self.dc_block == old.dc_block {
            self.previous_input = old.previous_input;
            self.previous_output = old.previous_output;
            self.prime = old.prime;
        } else {
            self.reset_state();
            // A newly enabled blocker starts at the current offset rather than
            // interpreting the already-present offset as a new audio impulse.
            self.prime = self.dc_block;
        }
    }
    pub(crate) fn reset_state(&mut self) {
        self.previous_input = [0.0; 2];
        self.previous_output = [0.0; 2];
        self.prime = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn model(dc: bool, rate: u32) -> StereoOutput {
        let node: Node = serde_json::from_value(
            json!({"id":"out","kind":"audio_stereo","params":{"dc_block":u8::from(dc)}}),
        )
        .unwrap();
        StereoOutput::build(&node, rate).unwrap()
    }
    #[test]
    fn disabled_is_exact_legacy_arithmetic_and_normaling() {
        let mut output = model(false, 48000);
        let mut left = PortFrame {
            channels: 4,
            ..PortFrame::default()
        };
        left.values[..4].copy_from_slice(&[1.25, -0.3, 8.0, 0.15]);
        let expected = left.values[..4].iter().sum::<f32>() / 10.0;
        let frame = output.process(left, PortFrame::default());
        assert_eq!(frame.channels, 2);
        assert_eq!(frame.values[0], expected);
        assert_eq!(frame.values[1], expected);
        assert!(frame.values[2..].iter().all(|v| *v == 0.0));
        let frame = output.process(left, PortFrame::mono(0.0));
        assert_eq!(frame.values[0], expected);
        assert_eq!(frame.values[1], 0.0);
    }
    #[test]
    fn dc_settles_and_stereo_histories_are_independent() {
        for rate in [8000, 44100, 48000, 192000] {
            let mut output = model(true, rate);
            let mut frame = PortFrame::default();
            for _ in 0..rate {
                frame = output.process(PortFrame::mono(10.0), PortFrame::mono(-5.0));
                assert!((frame.values[0] + 2.0 * frame.values[1]).abs() < 1e-6);
            }
            assert!(frame.values[0].abs() < 1e-12);
            let frame = output.process(PortFrame::mono(10.0), PortFrame::mono(0.0));
            assert!(frame.values[0].abs() < 1e-12);
            assert!(frame.values[1] > 0.49);
        }
    }
    #[test]
    fn calibrated_frequency_response_at_corner_and_bass() {
        for rate in [8000, 48000, 192000] {
            for hz in [10.0, 46.0] {
                let mut output = model(true, rate);
                let mut input_power = 0.0;
                let mut output_power = 0.0;
                for n in 0..2 * rate {
                    let value = (2.0 * PI * hz * f64::from(n) / f64::from(rate)).sin() as f32;
                    let frame = output.process(PortFrame::mono(value * 10.0), PortFrame::default());
                    assert_eq!(frame.values[0], frame.values[1]);
                    if n >= rate {
                        input_power += f64::from(value).powi(2);
                        output_power += f64::from(frame.values[0]).powi(2);
                    }
                }
                let gain = (output_power / input_power).sqrt();
                let expected = hz / (hz * hz + 100.0).sqrt();
                assert!(
                    (gain - expected).abs() < 0.0001,
                    "{rate} Hz / {hz} Hz: {gain}"
                );
            }
        }
    }
    #[test]
    fn adoption_reset_and_toggle_have_defined_history() {
        let mut old = model(true, 48000);
        for n in 0..2000 {
            old.process(
                PortFrame::mono((n as f32 * 0.03).sin()),
                PortFrame::default(),
            );
        }
        let mut adopted = model(true, 48000);
        adopted.adopt(&old);
        for _ in 0..200 {
            assert_eq!(
                adopted.process(PortFrame::mono(2.0), PortFrame::mono(-1.0)),
                old.process(PortFrame::mono(2.0), PortFrame::mono(-1.0))
            );
        }
        adopted.reset_state();
        assert_eq!(
            adopted.process(PortFrame::mono(2.0), PortFrame::default()),
            model(true, 48000).process(PortFrame::mono(2.0), PortFrame::default())
        );
        let disabled = model(false, 48000);
        let mut enabled = model(true, 48000);
        enabled.adopt(&disabled);
        assert_eq!(
            enabled
                .process(PortFrame::mono(10.0), PortFrame::mono(-3.0))
                .values,
            [0.0; 16]
        );
        let mut disabled = model(false, 48000);
        disabled.adopt(&enabled);
        assert_eq!(
            disabled
                .process(PortFrame::mono(10.0), PortFrame::default())
                .values[0],
            1.0
        );
        let mut rate_changed = model(true, 44100);
        rate_changed.adopt(&old);
        assert_eq!(
            rate_changed
                .process(PortFrame::mono(10.0), PortFrame::default())
                .values,
            [0.0; 16]
        );
    }
    #[test]
    fn invalid_flags_rejected_and_hostile_input_is_finite() {
        for params in [
            json!({"dc_block":0.5}),
            json!({"dc_block":2}),
            json!({"typo":1}),
        ] {
            let node: Node =
                serde_json::from_value(json!({"id":"out","kind":"audio_stereo","params":params}))
                    .unwrap();
            assert!(StereoOutput::build(&node, 48000).is_err());
        }
        for dc in [false, true] {
            let mut output = model(dc, 48000);
            for value in [
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::MAX,
                -f32::MAX,
                0.0,
            ] {
                assert!(output
                    .process(PortFrame::mono(value), PortFrame::default())
                    .values
                    .iter()
                    .all(|v| v.is_finite()));
            }
        }
    }
}
