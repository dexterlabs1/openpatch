//! Persistent runtime state, separate from transient physical controls.
use crate::Node;
use std::collections::BTreeMap;

// Callback state transfer must remain a bounded Copy operation without owning
// heap allocations. Collections of these states are boxed on the control thread
// to keep large fixed arrays off callback and test-thread stacks.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedModuleState {
    Seq3Run(bool),
    PushHold(bool),
    /// IEEE-754 voltage bits preserve exact equality in history and dirty checks.
    RandomValues {
        values: [[u32; 16]; 7],
        rng: u32,
    },
}
impl SavedModuleState {
    pub fn from_node(node: &Node) -> Option<Self> {
        match node.kind.as_str() {
            "seq3" => Some(Self::Seq3Run(
                node.params.get("running").copied().unwrap_or(1.0) != 0.0,
            )),
            "push" => Some(Self::PushHold(
                node.params.get("holding").copied().unwrap_or(0.0) != 0.0,
            )),
            "random_values" => crate::initial_random_values_state(node).ok(),
            _ => None,
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::Seq3Run(_) => "seq3",
            Self::PushHold(_) => "push",
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
            Self::RandomValues { .. } => None,
        }
    }
    pub fn value(self) -> bool {
        self.boolean().is_some_and(|(_, value)| value)
    }
    pub fn with_value(self, value: bool) -> Self {
        match self {
            Self::Seq3Run(_) => Self::Seq3Run(value),
            Self::PushHold(_) => Self::PushHold(value),
            Self::RandomValues { .. } => self,
        }
    }
    /// Overlay all persistent state on a graph or preset without changing controls.
    pub fn write_params(self, params: &mut BTreeMap<String, f32>) {
        match self {
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
