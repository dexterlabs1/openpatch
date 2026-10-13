//! Persistent runtime latches, separate from transient physical controls.
use crate::Node;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedModuleState {
    Seq3Run(bool),
    PushHold(bool),
}
impl SavedModuleState {
    /// Configured state before a live engine exists. Only validated 0/1
    /// parameters are meaningful; omitted Run defaults on and HOLD defaults off.
    pub fn from_node(node: &Node) -> Option<Self> {
        match node.kind.as_str() {
            "seq3" => Some(Self::Seq3Run(
                node.params.get("running").copied().unwrap_or(1.0) != 0.0,
            )),
            "push" => Some(Self::PushHold(
                node.params.get("holding").copied().unwrap_or(0.0) != 0.0,
            )),
            _ => None,
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::Seq3Run(_) => "seq3",
            Self::PushHold(_) => "push",
        }
    }
    pub fn parameter(self) -> &'static str {
        match self {
            Self::Seq3Run(_) => "running",
            Self::PushHold(_) => "holding",
        }
    }
    pub fn value(self) -> bool {
        match self {
            Self::Seq3Run(value) | Self::PushHold(value) => value,
        }
    }
    pub fn with_value(self, value: bool) -> Self {
        match self {
            Self::Seq3Run(_) => Self::Seq3Run(value),
            Self::PushHold(_) => Self::PushHold(value),
        }
    }
}
