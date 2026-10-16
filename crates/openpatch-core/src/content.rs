//! Persisted, control-thread content for modules without a signal path.
use crate::Node;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const MAX_NOTES_BYTES: usize = 8192;
pub const NOTES_WIDTH_HP: u16 = 16;
pub const MIN_BLANK_WIDTH_HP: u16 = 3;
pub const MAX_BLANK_WIDTH_HP: u16 = 96;
pub const DEFAULT_BLANK_WIDTH_HP: u16 = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModuleContent {
    Notes {
        text: String,
    },
    Blank {
        width_hp: u16,
    },
    /// Immutable samples live once in Patch.assets, referenced by SHA256.
    Wavetable {
        asset_id: String,
        wave_points: u32,
        name: String,
    },
}

impl Node {
    /// Notes text, including the empty default when content is absent.
    pub fn effective_notes_text(&self) -> Option<&str> {
        if self.kind != "notes" {
            return None;
        }
        Some(match &self.content {
            Some(ModuleContent::Notes { text }) => text,
            _ => "",
        })
    }

    /// Authoritative Blank width, including the default for older/compact JSON.
    pub fn effective_blank_width_hp(&self) -> Option<u16> {
        if self.kind != "blank" {
            return None;
        }
        Some(match self.content {
            Some(ModuleContent::Blank { width_hp }) => width_hp,
            _ => DEFAULT_BLANK_WIDTH_HP,
        })
    }
}

/// Validate typed content against its owning kind and host limits. Patch
/// compilation additionally checks that content and annotation kinds use v2.
pub fn validate_content(node: &Node) -> Result<(), String> {
    match (&node.content, node.kind.as_str()) {
        (None, _) => Ok(()),
        (Some(ModuleContent::Notes { text }), "notes") => {
            if text.len() > MAX_NOTES_BYTES {
                Err(format!(
                    "node '{}': Notes text exceeds {MAX_NOTES_BYTES} UTF-8 bytes",
                    node.id
                ))
            } else {
                Ok(())
            }
        }
        (Some(ModuleContent::Blank { width_hp }), "blank") => {
            if !(MIN_BLANK_WIDTH_HP..=MAX_BLANK_WIDTH_HP).contains(width_hp) {
                Err(format!("node '{}': Blank width_hp must be in {MIN_BLANK_WIDTH_HP}..={MAX_BLANK_WIDTH_HP}", node.id))
            } else {
                Ok(())
            }
        }
        (
            Some(ModuleContent::Wavetable {
                asset_id,
                wave_points,
                name,
            }),
            "wavetable_vco" | "wavetable_lfo",
        ) => {
            if asset_id.len() != 64
                || !asset_id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(format!(
                    "node '{}': invalid wavetable asset digest",
                    node.id
                ));
            }
            if name.len() > 1024 || name.contains('\0') {
                return Err(format!(
                    "node '{}': wavetable name exceeds 1024 bytes or contains NUL",
                    node.id
                ));
            }
            if *wave_points < 2
                || (node.kind == "wavetable_vco"
                    && (*wave_points < 32 || !wave_points.is_power_of_two()))
            {
                return Err(format!("node '{}': invalid wavetable wave length", node.id));
            }
            Ok(())
        }
        (Some(_), _) => Err(format!(
            "node '{}': content type does not match module kind '{}'",
            node.id, node.kind
        )),
    }
}

pub(crate) fn is_annotation(kind: &str) -> bool {
    matches!(kind, "notes" | "blank")
}

pub(crate) fn catalog() -> [Value; 2] {
    [
        json!({"kind":"notes","title":"NOTES","model":"annotation","description":"Editable multiline Unicode notes; no signal ports or DSP state", "inputs":[],"params":{},"width_hp":NOTES_WIDTH_HP,"content_schema":{"type":"object","additionalProperties":false,"required":["type","text"],"properties":{"type":{"const":"notes"},"text":{"type":"string","default":"","max_utf8_bytes":MAX_NOTES_BYTES}},"default":{"type":"notes","text":""}},"randomize":false}),
        json!({"kind":"blank","title":"BLANK","model":"annotation","description":"Resizable blank panel; no signal ports or DSP state", "inputs":[],"params":{},"width_hp":DEFAULT_BLANK_WIDTH_HP,"content_schema":{"type":"object","additionalProperties":false,"required":["type","width_hp"],"properties":{"type":{"const":"blank"},"width_hp":{"type":"integer","minimum":MIN_BLANK_WIDTH_HP,"maximum":MAX_BLANK_WIDTH_HP,"default":DEFAULT_BLANK_WIDTH_HP}},"default":{"type":"blank","width_hp":DEFAULT_BLANK_WIDTH_HP}},"randomize":false}),
    ]
}
