//! Persistent runtime state, separate from transient physical controls.
use crate::Node;
use std::collections::BTreeMap;

// Callback state transfer must remain a bounded Copy operation without owning
// heap allocations. Collections of these states are boxed on the control thread
// to keep large fixed arrays off callback and test-thread stacks.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedModuleState {
    MidiCc {
        assignments: [i16; 16],
        values: [u8; 128],
    },
    MidiCcMappings([i16; 16]),
    MidiGateMappings([i16; 16]),
    Seq3Run(bool),
    StockQuantizerNotes(u16),
    StockSeq3 {
        running: bool,
        gates: u8,
        passthrough: bool,
    },
    /// Selective persistent assignments; absent fields retain callback state.
    StockSeq3Fields {
        running: Option<bool>,
        gates: u8,
        gate_mask: u8,
        passthrough: Option<bool>,
    },
    PushHold(bool),
    /// IEEE-754 voltage bits preserve exact equality in history and dirty checks.
    ShiftRegister {
        values: [u32; 8],
        rng: u32,
    },
    RandomValues {
        values: [[u32; 16]; 7],
        rng: u32,
    },
}
impl SavedModuleState {
    pub fn from_node(node: &Node) -> Option<Self> {
        match node.kind.as_str() {
            "midi_gate" => crate::initial_midi_gate_state(node).ok(),
            "midi_cc" => crate::initial_midi_cc_state(node).ok(),
            "stock_seq3" => crate::initial_stock_seq3_state(node).ok(),
            "stock_quantizer" => {
                let mask = node.params.get("scale_mask").copied().unwrap_or(4095.0);
                (mask.is_finite() && mask.fract() == 0.0 && (0.0..=4095.0).contains(&mask))
                    .then_some(Self::StockQuantizerNotes(mask as u16))
            }
            "seq3" => Some(Self::Seq3Run(
                node.params.get("running").copied().unwrap_or(1.0) != 0.0,
            )),
            "push" => Some(Self::PushHold(
                node.params.get("holding").copied().unwrap_or(0.0) != 0.0,
            )),
            "sh_asr" => crate::initial_shift_register_state(node).ok(),
            "random_values" => crate::initial_random_values_state(node).ok(),
            _ => None,
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::MidiCc { .. } | Self::MidiCcMappings(_) => "midi_cc",
            Self::MidiGateMappings(_) => "midi_gate",
            Self::Seq3Run(_) => "seq3",
            Self::StockQuantizerNotes(_) => "stock_quantizer",
            Self::StockSeq3 { .. } | Self::StockSeq3Fields { .. } => "stock_seq3",
            Self::PushHold(_) => "push",
            Self::ShiftRegister { .. } => "sh_asr",
            Self::RandomValues { .. } => "random_values",
        }
    }
    /// Legacy boolean-state accessor. Generic callers should use `write_params`.
    pub fn parameter(self) -> &'static str {
        self.boolean().map(|(name, _)| name).unwrap_or("")
    }
    pub fn boolean(self) -> Option<(&'static str, bool)> {
        match self {
            Self::Seq3Run(value) => Some(("running", value)),
            Self::PushHold(value) => Some(("holding", value)),
            Self::StockSeq3 { .. }
            | Self::StockSeq3Fields { .. }
            | Self::StockQuantizerNotes(_)
            | Self::MidiCc { .. }
            | Self::MidiCcMappings(_)
            | Self::MidiGateMappings(_)
            | Self::ShiftRegister { .. }
            | Self::RandomValues { .. } => None,
        }
    }
    pub fn value(self) -> bool {
        self.boolean().is_some_and(|(_, value)| value)
    }
    pub fn with_value(self, value: bool) -> Self {
        match self {
            Self::Seq3Run(_) => Self::Seq3Run(value),
            Self::PushHold(_) => Self::PushHold(value),
            Self::StockSeq3 { .. }
            | Self::StockSeq3Fields { .. }
            | Self::StockQuantizerNotes(_)
            | Self::MidiCc { .. }
            | Self::MidiCcMappings(_)
            | Self::MidiGateMappings(_)
            | Self::ShiftRegister { .. }
            | Self::RandomValues { .. } => self,
        }
    }
    /// Overlay all persistent state on a graph or preset without changing controls.
    pub fn write_params(self, params: &mut BTreeMap<String, f32>) {
        match self {
            Self::StockQuantizerNotes(mask) => {
                params.insert("scale_mask".into(), f32::from(mask));
            }
            Self::StockSeq3Fields {
                running,
                gates,
                gate_mask,
                passthrough,
            } => {
                if let Some(value) = running {
                    params.insert("running".into(), f32::from(u8::from(value)));
                }
                if let Some(value) = passthrough {
                    params.insert("passthrough".into(), f32::from(u8::from(value)));
                }
                for i in 0..8 {
                    if gate_mask & (1 << i) != 0 {
                        params.insert(
                            format!("gate{}", i + 1),
                            f32::from(u8::from(gates & (1 << i) != 0)),
                        );
                    }
                }
            }
            Self::StockSeq3 {
                running,
                gates,
                passthrough,
            } => {
                params.insert("running".into(), f32::from(u8::from(running)));
                params.insert("passthrough".into(), f32::from(u8::from(passthrough)));
                for i in 0..8 {
                    params.insert(
                        format!("gate{}", i + 1),
                        f32::from(u8::from(gates & (1 << i) != 0)),
                    );
                }
            }
            Self::MidiCc {
                assignments,
                values,
            } => {
                Self::MidiCcMappings(assignments).write_params(params);
                for (i, value) in values.iter().enumerate() {
                    params.insert(format!("cc_value{i}"), *value as f32);
                }
            }
            Self::MidiGateMappings(assignments) => {
                for (i, value) in assignments.iter().enumerate() {
                    params.insert(format!("note{}", i + 1), *value as f32);
                }
            }
            Self::MidiCcMappings(assignments) => {
                for (i, value) in assignments.iter().enumerate() {
                    params.insert(format!("cc{}", i + 1), *value as f32);
                }
            }
            Self::ShiftRegister { values, rng } => {
                for (row, value) in values.iter().enumerate() {
                    params.insert(format!("value{}", row + 1), f32::from_bits(*value));
                }
                params.insert("rng_hi".into(), (rng >> 16) as f32);
                params.insert("rng_lo".into(), (rng & 0xffff) as f32);
            }
            Self::RandomValues { values, rng } => {
                for (output, row) in values.iter().enumerate() {
                    for (channel, value) in row.iter().enumerate() {
                        params.insert(
                            format!("value{}_{}", output + 1, channel + 1),
                            f32::from_bits(*value),
                        );
                    }
                }
                params.insert("rng_hi".into(), (rng >> 16) as f32);
                params.insert("rng_lo".into(), (rng & 0xffff) as f32);
            }
            _ => {
                if let Some((parameter, value)) = self.boolean() {
                    params.insert(parameter.into(), if value { 1.0 } else { 0.0 });
                }
            }
        }
    }
}
