//! Original fixed-storage assembler for sixteen first-lane inputs.
use crate::{parameter, ModuleLightState, Node, PortFrame};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChannelSelection {
    Auto,
    Fixed(u8),
}

impl ChannelSelection {
    fn parse(node: &Node) -> Result<Self, String> {
        // This saved OpenPatch encoding deliberately preserves legacy zero=Auto.
        // Validate the signed sentinel before converting only positive counts.
        let value = parameter(node, "channels", 0.0, -1.0, 16.0)?;
        if value.fract() != 0.0 {
            return Err(format!(
                "node '{}': parameter 'channels' must be an integer",
                node.id
            ));
        }
        Ok(if value == 0.0 {
            Self::Auto
        } else if value == -1.0 {
            Self::Fixed(0)
        } else {
            Self::Fixed(value as u8)
        })
    }
}

#[derive(Debug)]
pub(crate) struct Merge {
    selection: ChannelSelection,
    output_channels: u8,
    automatic_channels: u8,
}

impl Merge {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        Ok(Self {
            selection: ChannelSelection::parse(node)?,
            output_channels: 0,
            automatic_channels: 0,
        })
    }

    pub(crate) fn process(&mut self, inputs: &[PortFrame; 16]) -> PortFrame {
        let mut frame = PortFrame::default();
        let mut automatic_channels = 0;
        for (slot, input) in inputs.iter().enumerate() {
            if input.channels > 0 {
                automatic_channels = (slot + 1) as u8;
                frame.values[slot] = input.values[0];
            }
        }
        frame.channels = match self.selection {
            ChannelSelection::Auto => automatic_channels,
            ChannelSelection::Fixed(count) => count,
        };
        // OpenPatch's inactive output slots are always zero, including forced zero.
        frame.values[usize::from(frame.channels)..].fill(0.0);
        self.output_channels = frame.channels;
        self.automatic_channels = automatic_channels;
        frame
    }

    pub(crate) fn adopt(&mut self, previous: &Self) {
        // The receiving engine's newly compiled selection remains authoritative.
        self.output_channels = previous.output_channels;
        self.automatic_channels = previous.automatic_channels;
    }

    pub(crate) fn initialize_state(&mut self) {
        self.selection = ChannelSelection::Auto;
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            output_channels: Some(self.output_channels),
            automatic_channels: Some(self.automatic_channels),
            ..ModuleLightState::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(channels: Option<f32>) -> Merge {
        Merge::build(&Node {
            id: "merge".into(),
            kind: "poly_merge".into(),
            params: channels
                .map(|v| [("channels".into(), v)].into())
                .unwrap_or_default(),
            bypassed: false,
            content: None,
            score: None,
        })
        .unwrap()
    }

    #[test]
    fn sparse_inputs_use_only_first_lanes_and_actual_counts() {
        let mut merge = merge(None);
        let mut inputs = [PortFrame::default(); 16];
        // Inactive data, including in the highest slot, is never occupancy.
        inputs[15].values.fill(99.0);
        assert_eq!(merge.process(&inputs), PortFrame::default());
        inputs[2] = PortFrame::mono(-3.0);
        inputs[2].values[1] = 123.0;
        inputs[7] = PortFrame {
            channels: 16,
            values: [0.0; 16],
        };
        inputs[7].values[1] = 71.0;
        let output = merge.process(&inputs);
        assert_eq!(output.channels, 8);
        assert_eq!(output.values[2], -3.0);
        assert!(output
            .values
            .iter()
            .enumerate()
            .all(|(i, v)| i == 2 || *v == 0.0));
        assert_eq!(merge.lights().automatic_channels, Some(8));
        assert_eq!(merge.lights().output_channels, Some(8));
        inputs[15] = PortFrame::mono(0.0);
        assert_eq!(merge.process(&inputs).channels, 16);
        assert_eq!(merge.lights().automatic_channels, Some(16));
    }

    #[test]
    fn forced_counts_clear_inactive_lanes_and_auto_shrinks_and_regrows() {
        let inputs = std::array::from_fn(|i| PortFrame::mono(i as f32 + 1.0));
        for (selection, count) in [(-1.0, 0), (1.0, 1), (7.0, 7), (16.0, 16)] {
            let mut merge = merge(Some(selection));
            let output = merge.process(&inputs);
            assert_eq!(output.channels, count);
            assert_eq!(merge.lights().automatic_channels, Some(16));
            assert_eq!(merge.lights().output_channels, Some(count));
            assert!(output.values[usize::from(count)..]
                .iter()
                .all(|v| *v == 0.0));
            for (i, v) in output.values[..usize::from(count)].iter().enumerate() {
                assert_eq!(*v, i as f32 + 1.0);
            }
        }
        let mut merge = merge(Some(0.0));
        assert_eq!(merge.process(&inputs).channels, 16);
        let mut shrunk = [PortFrame::default(); 16];
        shrunk[0] = PortFrame::mono(-2.0);
        assert_eq!(merge.process(&shrunk), PortFrame::mono(-2.0));
        assert_eq!(
            merge.process(&[PortFrame::default(); 16]),
            PortFrame::default()
        );
        assert_eq!(
            merge.process(&inputs).values,
            std::array::from_fn(|i| i as f32 + 1.0)
        );
    }

    #[test]
    fn adoption_and_initialize_preserve_observations_but_initialize_selects_auto() {
        let mut previous = merge(Some(7.0));
        let mut inputs = [PortFrame::default(); 16];
        inputs[15] = PortFrame::mono(5.0);
        previous.process(&inputs);
        let mut receiving = merge(Some(-1.0));
        receiving.adopt(&previous);
        assert_eq!(receiving.lights(), previous.lights());
        assert_eq!(receiving.process(&inputs), PortFrame::default());
        let before = receiving.lights();
        receiving.initialize_state();
        assert_eq!(receiving.lights(), before);
        assert_eq!(receiving.process(&inputs).channels, 16);
        let lights = receiving.lights();
        assert_eq!(lights.count, 0);
        assert_eq!(lights.brightness, [[0.0; 2]; crate::MAX_MODULE_LIGHTS]);
    }
}
