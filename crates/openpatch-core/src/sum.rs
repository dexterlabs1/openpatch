//! Original mono polyphonic summer and post-level peak meter.
use crate::{bounded, parameter, ModuleLightState, Node, PortFrame};

#[derive(Debug)]
pub(crate) struct Sum {
    level: f32,
    release: f32,
    peak: f32,
    channels: u8,
    divider: u16,
    brightness: [f32; 6],
}
impl Sum {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        Ok(Self {
            level: parameter(node, "level", 1.0, 0.0, 1.0)?,
            release: 10.0 * ((1.0 / rate as f32) * 16.0),
            peak: 0.0,
            channels: 0,
            divider: 0,
            brightness: [0.0; 6],
        })
    }
    pub(crate) fn process(&mut self, input: PortFrame) -> PortFrame {
        // Preserve the existing active-lane f32 sum, applying gain before safety limiting.
        let total: f32 = input.values[..usize::from(input.channels.min(16))]
            .iter()
            .sum();
        let output = bounded(total * self.level);
        self.divider = (self.divider + 1) % 512;
        if self.divider.is_multiple_of(16) {
            let amplitude = output.abs() / 10.0;
            if amplitude >= self.peak {
                self.peak = amplitude;
            } else {
                self.peak += (amplitude - self.peak) * self.release;
            }
        }
        if self.divider == 0 {
            self.channels = input.channels.min(16);
            let db = 20.0 * self.peak.log10();
            for (index, (low, high)) in [
                (0.0, 0.0),
                (-3.0, 0.0),
                (-6.0, -3.0),
                (-12.0, -6.0),
                (-24.0, -12.0),
                (-36.0, -24.0),
            ]
            .into_iter()
            .enumerate()
            {
                self.brightness[index] = if db >= high {
                    1.0
                } else if db <= low {
                    0.0
                } else {
                    (db - low) / (high - low)
                };
            }
        }
        PortFrame::mono(output)
    }
    pub(crate) fn adopt(&mut self, previous: &Self) {
        self.peak = previous.peak;
        self.channels = previous.channels;
        self.divider = previous.divider;
        self.brightness = previous.brightness;
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            octave_shift: None,
            count: 6,
            input_channels: Some(self.channels),
            output_channels: None,
            automatic_channels: None,
            clock_frequency: None,
            playing_notes: None,
            waveform_position: None,
            brightness: std::array::from_fn(|i| {
                [self.brightness.get(i).copied().unwrap_or(0.0), 0.0]
            }),
            manual: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sum(level: f32, rate: u32) -> Sum {
        Sum::build(
            &Node {
                id: "sum".into(),
                kind: "poly_sum".into(),
                params: [("level".into(), level)].into(),
                bypassed: false,
                content: None,
                rackmcp_sequence: None,
                score: None,
            },
            rate,
        )
        .unwrap()
    }
    #[test]
    fn counts_signed_sum_gain_and_safety() {
        let mut s = sum(0.25, 48000);
        for count in [0, 1, 16] {
            let mut input = PortFrame {
                channels: count,
                ..PortFrame::default()
            };
            input.values.fill(8.0);
            let out = s.process(input);
            assert_eq!(out.channels, 1);
            assert_eq!(out.values[0], f32::from(count) * 2.0);
            for _ in 0..512 {
                s.process(input);
            }
            assert_eq!(s.lights().input_channels, Some(count));
        }
        let mut input = PortFrame::mono(16.0);
        input.channels = 2;
        input.values[1] = -16.0;
        assert_eq!(s.process(input).values[0], 0.0);
        input.values[0] = 2_000_000.0;
        input.values[1] = 2_000_000.0;
        assert_eq!(s.process(input).values[0], 1_000_000.0);
    }
    #[test]
    fn meter_dividers_reference_bands_and_release() {
        let mut s = sum(0.5, 48000);
        for _ in 0..511 {
            s.process(PortFrame::mono(10.0));
        }
        assert_eq!(s.lights().input_channels, Some(0));
        assert_eq!(s.lights().brightness, [[0.0; 2]; crate::MAX_MODULE_LIGHTS]);
        s.process(PortFrame::mono(10.0));
        let l = s.lights();
        assert_eq!(l.input_channels, Some(1));
        assert_eq!(l.brightness[0][0], 0.0);
        assert_eq!(l.brightness[1][0], 0.0);
        assert_eq!(l.brightness[2][0], 0.0);
        assert!((l.brightness[3][0] - 0.99656665).abs() < 1e-6);
        assert_eq!(l.brightness[4][0], 1.0);
        for _ in 0..4800 {
            s.process(PortFrame::mono(0.0));
        }
        let expected = 0.5 * (1.0_f32 - 160.0 / 48000.0).powi(300);
        assert!((s.peak - expected).abs() < 2e-6);
    }
    #[test]
    fn private_adoption_keeps_meter_phase_and_parameters_outer_engine_handles_rate_changes() {
        let mut old = sum(1.0, 48000);
        for _ in 0..511 {
            old.process(PortFrame::mono(10.0));
        }
        let mut new = sum(0.0, 96000);
        new.adopt(&old);
        assert_eq!(new.process(PortFrame::mono(10.0)).values[0], 0.0);
        assert_eq!(new.lights().input_channels, Some(1));
        assert!((new.peak - (1.0 - 160.0 / 96000.0)).abs() < 1e-6);
    }
}
