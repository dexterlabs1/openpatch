//! Persistent runtime state, separate from transient physical controls.
use crate::Node;
use std::collections::BTreeMap;

/// A source pattern name copied across callback boundaries without allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedPatternName {
    bytes: [u8; 32],
    len: u8,
}
impl FixedPatternName {
    pub fn new(name: &str) -> Option<Self> {
        if name.is_empty() || name.len() > 32 || name.contains('\0') {
            return None;
        }
        let mut bytes = [0; 32];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Some(Self {
            bytes,
            len: name.len() as u8,
        })
    }
    pub fn as_str(&self) -> &str {
        // Only the checked UTF-8 constructor can populate these private bytes.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("valid pattern name")
    }
}
impl serde::Serialize for FixedPatternName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> serde::Deserialize<'de> for FixedPatternName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NameVisitor;
        impl serde::de::Visitor<'_> for NameVisitor {
            type Value = FixedPatternName;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a nonempty UTF-8 pattern name of at most 32 bytes without NUL")
            }
            fn visit_str<E: serde::de::Error>(self, name: &str) -> Result<Self::Value, E> {
                FixedPatternName::new(name).ok_or_else(|| E::custom("invalid source pattern name"))
            }
        }
        deserializer.deserialize_str(NameVisitor)
    }
}

// Callback state transfer must remain a bounded Copy operation without owning
// heap allocations. Collections of these states are boxed on the control thread
// to keep large fixed arrays off callback and test-thread stacks.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedModuleState {
    RackMcpSelection(FixedPatternName),
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
            "rackmcp_sequencer" => {
                let name = match node.rackmcp_sequence.as_deref() {
                    Some(state) => state.data.selected_pattern.as_deref().or_else(|| {
                        state
                            .data
                            .patterns
                            .first()
                            .map(|pattern| pattern.name.as_str())
                    })?,
                    None => "A",
                };
                Some(Self::RackMcpSelection(FixedPatternName::new(name)?))
            }
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
            Self::RackMcpSelection(_) => "rackmcp_sequencer",
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
            | Self::RandomValues { .. }
            | Self::RackMcpSelection(_) => None,
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
            | Self::RandomValues { .. }
            | Self::RackMcpSelection(_) => self,
        }
    }
    /// Overlay persistent runtime state on its authored node. This runs on the
    /// control thread; structured sequence selection is not a parameter.
    pub fn write_node(self, node: &mut Node) {
        match self {
            Self::RackMcpSelection(name) => {
                let state = node.rackmcp_sequence.get_or_insert_with(|| {
                    Box::new(crate::rackmcp_sequence::SequenceState::default())
                });
                state.data.selected_pattern = Some(name.as_str().into());
            }
            _ => self.write_params(&mut node.params),
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
