//! Original fixed-storage splitter with passive processed input-count observation.
use crate::{ModuleLightState, PortFrame, MAX_CHANNELS};

#[derive(Debug, Default)]
pub(crate) struct Split {
    input_channels: u8,
}

impl Split {
    pub(crate) fn process(&mut self, input: &PortFrame) -> [PortFrame; MAX_CHANNELS] {
        self.input_channels = input.channels.min(MAX_CHANNELS as u8);
        // The pinned Split class deliberately copies every raw slot, including
        // diagnostic values outside the input's declared width. Normal Engine
        // producers remain zero-padded; cable sanitization is a separate policy.
        std::array::from_fn(|channel| PortFrame::mono(input.values[channel]))
    }

    pub(crate) fn adopt(&mut self, previous: &Self) {
        self.input_channels = previous.input_channels;
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            input_channels: Some(self.input_channels),
            ..ModuleLightState::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_slot_copy_preserves_bits_without_clipping_or_sanitizing() {
        let values = [
            0.0,
            -0.0,
            16.0,
            -16.0,
            f32::MAX,
            -f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::from_bits(0x7fc1_2345),
            f32::from_bits(0xffc5_4321),
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            f32::from_bits(1),
            f32::from_bits(0x8000_0001),
            0.5,
            -0.5,
        ];
        let mut split = Split::default();
        for channels in 0..=MAX_CHANNELS as u8 {
            let output = split.process(&PortFrame { channels, values });
            assert_eq!(split.lights().input_channels, Some(channels));
            for (channel, frame) in output.iter().enumerate() {
                assert_eq!(frame.channels, 1);
                assert_eq!(frame.values[0].to_bits(), values[channel].to_bits());
                assert!(frame.values[1..].iter().all(|value| value.to_bits() == 0));
            }
        }
    }

    #[test]
    fn valid_frames_split_every_width_without_broadcast_or_signal_lamps() {
        let mut split = Split::default();
        assert_eq!(split.lights().input_channels, Some(0));
        for channels in 0..=MAX_CHANNELS as u8 {
            let mut input = PortFrame {
                channels,
                ..PortFrame::default()
            };
            for (channel, sample) in input.values[..usize::from(channels)].iter_mut().enumerate() {
                *sample = match channel {
                    0 => -0.0,
                    1 => 0.0,
                    _ => channel as f32 - 8.0,
                };
            }
            let output = split.process(&input);
            for (channel, frame) in output.iter().enumerate() {
                assert_eq!(frame.channels, 1);
                assert_eq!(frame.values[0].to_bits(), input.values[channel].to_bits());
                assert!(frame.values[1..].iter().all(|value| value.to_bits() == 0));
            }
            let lights = split.lights();
            assert_eq!(lights.input_channels, Some(channels));
            assert_eq!(lights.count, 0);
            assert_eq!(lights.brightness, [[0.0; 2]; crate::MAX_MODULE_LIGHTS]);
            assert_eq!(lights.output_channels, None);
            assert_eq!(lights.automatic_channels, None);
            assert!(!lights.manual);
        }
        assert_eq!(
            split.process(&PortFrame::default()),
            [PortFrame::mono(0.0); MAX_CHANNELS]
        );
        assert_eq!(split.lights().input_channels, Some(0));
    }
}
