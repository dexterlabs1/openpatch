//! Canonical raw host-port storage for native stock models. Legacy DSP keeps
//! its public logical frames; its producer adapter writes this common store.
//! Every v2 graph retains this history before and after native consumers exist.
//! Storage costs sixteen raw input ports per module and one raw port per output
//! (each port contains sixteen f32 slots and a channel count), plus contexts and
//! compiled connected-group indices. Only connected input groups transfer each
//! frame. No performance improvement or device behavior is claimed.
use crate::{InputLifecycleTransform, OutputLifecycleTransform, PortFrame, MAX_CHANNELS};

pub(crate) const PORTS: usize = 16;
pub(crate) const OUTPUT_PORTS: usize = 32;

/// A passive copy of actual private storage, including inactive raw lanes.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct HostPortSnapshot {
    pub channels: u8,
    pub values: [f32; MAX_CHANNELS],
}

impl HostPortSnapshot {
    /// SDK Output::setChannels: disconnected storage remains disconnected;
    /// otherwise clear the retired prefix before coercing requested zero to one.
    pub(crate) fn set_channels(&mut self, requested: u8) {
        if self.channels == 0 {
            return;
        }
        let requested = requested.min(MAX_CHANNELS as u8);
        for lane in requested..self.channels {
            self.values[usize::from(lane)] = 0.0;
        }
        self.channels = requested.max(1);
    }

    pub(crate) fn frame(&self) -> PortFrame {
        let mut frame = PortFrame {
            channels: self.channels,
            ..PortFrame::default()
        };
        let count = usize::from(self.channels);
        frame.values[..count].copy_from_slice(&self.values[..count]);
        frame
    }

    /// Legacy producer policy: set desired logical width, then write all slots.
    /// Inactive public slots are zero; a connected desired-zero source is mono0.
    pub(crate) fn write_logical(&mut self, frame: &PortFrame) {
        self.set_channels(frame.channels);
        self.values = frame.values;
    }
}

/// Effects resolved to compiled local port indices before entering the callback.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostPortLifecycle {
    pub inputs: [InputLifecycleTransform; PORTS],
    pub outputs: [OutputLifecycleTransform; OUTPUT_PORTS],
    pub input_presence: u16,
    pub output_consumers: u32,
    pub bypassed: bool,
}

/// Passive whole-module state. Reading this never adds an output consumer.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct HostModuleSnapshot {
    pub inputs: [HostPortSnapshot; PORTS],
    pub outputs: [HostPortSnapshot; OUTPUT_PORTS],
    pub input_presence: u16,
    pub output_consumers: u32,
    pub bypassed: bool,
    pub input_count: u8,
    pub output_count: u8,
}

/// Control-thread inspection of actual compiled cable endpoints. This allocates
/// only when explicitly requested and never changes consumers or processes audio.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HostCableSnapshot {
    pub id: Option<String>,
    pub source: String,
    pub source_port: u8,
    pub destination: String,
    pub destination_port: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HostContext {
    pub input_presence: u16,
    pub output_consumers: u32,
    pub bypassed: bool,
}

#[derive(Debug)]
pub(crate) struct HostPorts {
    pub inputs: Vec<[HostPortSnapshot; PORTS]>,
    pub outputs: Vec<HostPortSnapshot>,
    pub contexts: Vec<HostContext>,
}

impl HostPorts {
    pub(crate) fn new(nodes: usize, outputs: usize) -> Self {
        Self {
            inputs: vec![[HostPortSnapshot::default(); PORTS]; nodes],
            outputs: vec![HostPortSnapshot::default(); outputs],
            contexts: vec![HostContext::default(); nodes],
        }
    }
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// Actual end-of-frame cable transfer; only connected groups are visited.
/// Preserve untouched raw slots beyond both the old and new channel counts.
/// Single cables copy finite bits unchanged; stacked f32 additions use the
/// Engine's signed FTZ/DAZ arithmetic environment, without changing global flags.
pub(crate) fn transfer_input(
    input: &mut HostPortSnapshot,
    outputs: &[HostPortSnapshot],
    sources: &[usize],
) {
    if sources.len() == 1 {
        let output = &outputs[sources[0]];
        if output.channels == 1 {
            input.values[0] = finite(output.values[0]);
            if input.channels > 1 {
                input.values[1..usize::from(input.channels)].fill(0.0);
            }
            input.channels = 1;
            return;
        }
        let channels = usize::from(output.channels);
        for (destination, &value) in input.values[..channels]
            .iter_mut()
            .zip(&output.values[..channels])
        {
            *destination = finite(value);
        }
        if input.channels > output.channels {
            input.values[channels..usize::from(input.channels)].fill(0.0);
        }
        input.channels = output.channels;
        return;
    }
    let channels = sources
        .iter()
        .map(|&index| outputs[index].channels)
        .max()
        .unwrap_or(0);
    let cleared = usize::from(channels.max(input.channels));
    input.values[..cleared].fill(0.0);
    input.channels = channels;
    for &index in sources {
        let output = &outputs[index];
        if output.channels == 1 {
            let value = finite(output.values[0]);
            for lane in &mut input.values[..usize::from(channels)] {
                *lane = crate::native_float::rack_add(*lane, value);
            }
        } else {
            for lane in 0..usize::from(output.channels) {
                input.values[lane] =
                    crate::native_float::rack_add(input.values[lane], finite(output.values[lane]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_slot_publication_matches_retirement_then_store_for_all_channel_counts_and_raw_bits() {
        let patterns = [
            0x0000_0000,
            0x8000_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
            0xffc5_4321,
            0x7f80_0001,
            0xff80_0001,
            0x7f7f_ffff,
            0xff7f_ffff,
            0x0000_0001,
            0x8000_0001,
            0x0080_0000,
            0x8080_0000,
            0x3f80_0000,
            0xbf80_0000,
        ];
        for previous_count in 0..=16 {
            for requested_count in 0..=16 {
                for rotation in 0..16 {
                    let frame = PortFrame {
                        channels: requested_count,
                        // Deliberately include arbitrary raw inactive slots.
                        values: std::array::from_fn(|lane| {
                            f32::from_bits(patterns[(lane + rotation) % 16])
                        }),
                    };
                    let initial = HostPortSnapshot {
                        channels: previous_count,
                        values: [f32::from_bits(0xffc0_9876); 16],
                    };
                    let mut original = initial;
                    original.set_channels(requested_count);
                    original.values = frame.values;
                    let mut optimized = initial;
                    optimized.write_logical(&frame);
                    assert_eq!(optimized.channels, original.channels);
                    assert_eq!(
                        optimized.values.map(f32::to_bits),
                        original.values.map(f32::to_bits)
                    );
                    assert_eq!(
                        optimized.values.map(f32::to_bits),
                        frame.values.map(f32::to_bits)
                    );
                    assert_eq!(
                        optimized.channels,
                        if previous_count == 0 {
                            0
                        } else {
                            requested_count.max(1)
                        }
                    );
                }
            }
        }
    }

    #[test]
    fn single_cable_transfer_matches_scalar_copy_and_retirement_for_all_counts_and_raw_tails() {
        let patterns = [
            0x0000_0000,
            0x8000_0000,
            0x7f80_0000,
            0xff80_0000,
            0x7fc1_2345,
            0xffc5_4321,
            0x7f80_0001,
            0xff80_0001,
            0x7f7f_ffff,
            0xff7f_ffff,
            0x0000_0001,
            0x8000_0001,
            0x0080_0000,
            0x8080_0000,
            0x3f80_0000,
            0xbf80_0000,
        ];
        for old_count in 0..=16_u8 {
            for new_count in 0..=16_u8 {
                for rotation in 0..16 {
                    let old_values =
                        std::array::from_fn(|lane| f32::from_bits(patterns[(lane + 7) % 16]));
                    let source = HostPortSnapshot {
                        channels: new_count,
                        values: std::array::from_fn(|lane| {
                            f32::from_bits(patterns[(lane + rotation) % 16])
                        }),
                    };
                    let mut expected = old_values;
                    // Independent scalar edge law: sanitize only copied active
                    // values, retire old active lanes, retain all other raw bits.
                    for (lane, destination) in
                        expected.iter_mut().enumerate().take(usize::from(new_count))
                    {
                        let value = source.values[lane];
                        *destination = if value.is_finite() { value } else { 0.0 };
                    }
                    for lane in new_count..old_count {
                        expected[usize::from(lane)] = 0.0;
                    }
                    let mut input = HostPortSnapshot {
                        channels: old_count,
                        values: old_values,
                    };
                    let decoy = HostPortSnapshot {
                        channels: 16,
                        values: [99.0; 16],
                    };
                    transfer_input(&mut input, &[decoy, source], &[1]);
                    assert_eq!(input.channels, new_count);
                    assert_eq!(input.values.map(f32::to_bits), expected.map(f32::to_bits));
                    let tail = usize::from(old_count.max(new_count));
                    for (actual, before) in input.values[tail..].iter().zip(&old_values[tail..]) {
                        assert_eq!(actual.to_bits(), before.to_bits());
                    }
                }
            }
        }
    }

    #[test]
    fn stacked_transfer_keeps_declared_source_order_and_clears_retired_active_lanes() {
        let outputs = [
            HostPortSnapshot {
                channels: 1,
                values: [1e20; 16],
            },
            HostPortSnapshot {
                channels: 1,
                values: [-1e20; 16],
            },
            HostPortSnapshot {
                channels: 1,
                values: [3.0; 16],
            },
        ];
        let mut first = HostPortSnapshot {
            channels: 16,
            values: [-0.0; 16],
        };
        let mut second = first;
        transfer_input(&mut first, &outputs, &[0, 1, 2]);
        transfer_input(&mut second, &outputs, &[0, 2, 1]);
        assert_eq!(first.channels, 1);
        assert_eq!(first.values[0], 3.0);
        assert_eq!(second.values[0], 0.0);
        assert!(first.values[1..].iter().all(|value| value.to_bits() == 0));
        assert!(second.values[1..].iter().all(|value| value.to_bits() == 0));
    }

    #[test]
    fn stacked_normal_cancellation_flushes_tiny_results_with_sign_and_preserves_tail() {
        let minimum = f32::MIN_POSITIVE;
        let next = f32::from_bits(0x0080_0001);
        let mut wide = HostPortSnapshot {
            channels: 2,
            ..Default::default()
        };
        wide.values[..2].copy_from_slice(&[minimum, next]);
        let mono = HostPortSnapshot {
            channels: 1,
            values: [-next; 16],
        };
        let tail = f32::from_bits(0xffc1_2345);
        let mut input = HostPortSnapshot {
            channels: 3,
            values: [tail; 16],
        };
        transfer_input(&mut input, &[wide, mono], &[0, 1]);
        assert_eq!(input.channels, 2);
        assert_eq!(input.values[0].to_bits(), (-0.0_f32).to_bits());
        assert_eq!(input.values[1].to_bits(), 0);
        assert_eq!(input.values[2].to_bits(), 0);
        assert!(input.values[3..]
            .iter()
            .all(|value| value.to_bits() == tail.to_bits()));
    }

    #[test]
    fn stacked_subnormal_operands_flush_before_mono_broadcast_addition() {
        for negative in [false, true] {
            let normal = if negative {
                -f32::MIN_POSITIVE
            } else {
                f32::MIN_POSITIVE
            };
            let mono = HostPortSnapshot {
                channels: 1,
                values: [normal; 16],
            };
            let mut poly = HostPortSnapshot {
                channels: 2,
                ..Default::default()
            };
            poly.values[..2].copy_from_slice(&[f32::from_bits(1), f32::from_bits(0x8000_0001)]);
            let mut input = HostPortSnapshot::default();
            transfer_input(&mut input, &[mono, poly], &[0, 1]);
            assert_eq!(input.channels, 2);
            assert_eq!(input.values[0].to_bits(), normal.to_bits());
            assert_eq!(input.values[1].to_bits(), normal.to_bits());
        }
    }

    #[test]
    fn stacked_mono_zero_signs_keep_source_order_after_underflow() {
        let minimum = f32::MIN_POSITIVE;
        let next = f32::from_bits(0x0080_0001);
        let outputs = [
            HostPortSnapshot {
                channels: 2,
                values: [minimum; 16],
            },
            HostPortSnapshot {
                channels: 1,
                values: [-next; 16],
            },
            HostPortSnapshot {
                channels: 1,
                values: [-0.0; 16],
            },
            HostPortSnapshot {
                channels: 1,
                values: [0.0; 16],
            },
        ];
        let mut negative = HostPortSnapshot::default();
        let mut positive = HostPortSnapshot::default();
        transfer_input(&mut negative, &outputs, &[0, 3, 1, 2]);
        transfer_input(&mut positive, &outputs, &[0, 1, 2, 3]);
        for lane in 0..2 {
            assert_eq!(negative.values[lane].to_bits(), (-0.0_f32).to_bits());
            assert_eq!(positive.values[lane].to_bits(), 0);
        }
    }

    #[test]
    fn single_cable_subnormal_slots_remain_raw_copy_bits() {
        let mut output = HostPortSnapshot {
            channels: 2,
            ..Default::default()
        };
        output.values[..2].copy_from_slice(&[f32::from_bits(1), f32::from_bits(0x8000_0001)]);
        let tail = f32::from_bits(0x8000_0042);
        let mut input = HostPortSnapshot {
            channels: 2,
            values: [tail; 16],
        };
        transfer_input(&mut input, &[output], &[0]);
        assert_eq!(input.channels, 2);
        assert_eq!(input.values[0].to_bits(), 1);
        assert_eq!(input.values[1].to_bits(), 0x8000_0001);
        assert!(input.values[2..]
            .iter()
            .all(|value| value.to_bits() == tail.to_bits()));
    }
}
