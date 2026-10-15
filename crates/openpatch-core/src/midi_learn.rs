//! Kind identity for the shared fixed-capacity MIDI Learn transport.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiLearnKind {
    Cc,
    Gate,
}
impl MidiLearnKind {
    pub const fn module_kind(self) -> &'static str {
        match self {
            Self::Cc => "midi_cc",
            Self::Gate => "midi_gate",
        }
    }
    pub fn from_module_kind(kind: &str) -> Option<Self> {
        match kind {
            "midi_cc" => Some(Self::Cc),
            "midi_gate" => Some(Self::Gate),
            _ => None,
        }
    }
}
/// The original CC completion layout remains the common callback-safe payload.
pub type MidiLearnCompletion = crate::MidiCcLearnCompletion;
