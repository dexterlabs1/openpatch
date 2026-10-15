//! Host-side cable colour intent. Saved routes always own their explicit RGB.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_PRESETS: usize = 12;
pub(crate) const MAX_COLORS: usize = 16;
pub(crate) const MAX_NAME_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preset {
    pub name: String,
    pub colors: Vec<[u8; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub color: [u8; 3],
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Palette {
    presets: Vec<Preset>,
    active: usize,
    next: usize,
    auto_advance: bool,
    generation: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u8,
    presets: Vec<Preset>,
    active: usize,
    next: usize,
    auto_advance: bool,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            presets: vec![
                Preset {
                    name: "Studio".into(),
                    colors: vec![
                        [211, 101, 83],
                        [223, 164, 67],
                        [72, 171, 167],
                        [75, 139, 189],
                        [169, 112, 173],
                        [195, 138, 80],
                    ],
                },
                Preset {
                    name: "Bright".into(),
                    colors: vec![
                        [244, 104, 111],
                        [255, 199, 80],
                        [106, 215, 157],
                        [91, 190, 240],
                        [184, 142, 237],
                        [244, 151, 92],
                    ],
                },
                Preset {
                    name: "Muted".into(),
                    colors: vec![
                        [177, 113, 107],
                        [182, 155, 103],
                        [111, 155, 143],
                        [117, 144, 172],
                        [155, 126, 166],
                        [167, 139, 116],
                    ],
                },
            ],
            active: 0,
            next: 0,
            auto_advance: true,
            generation: 0,
        }
    }
}

impl Palette {
    pub(crate) fn from_settings(value: &Value) -> Self {
        let Ok(settings) = serde_json::from_value::<Settings>(value.clone()) else {
            return Self::default();
        };
        if settings.version != 1
            || validate(&settings.presets, settings.active, settings.next).is_err()
        {
            return Self::default();
        }
        Self {
            presets: settings.presets,
            active: settings.active,
            next: settings.next,
            auto_advance: settings.auto_advance,
            generation: 0,
        }
    }

    pub(crate) fn settings(&self) -> Value {
        serde_json::to_value(Settings {
            version: 1,
            presets: self.presets.clone(),
            active: self.active,
            next: self.next,
            auto_advance: self.auto_advance,
        })
        .expect("RGB palette is serializable")
    }

    pub(crate) fn presets(&self) -> &[Preset] {
        &self.presets
    }
    pub(crate) fn active_index(&self) -> usize {
        self.active
    }
    pub(crate) fn next_index(&self) -> usize {
        self.next
    }
    pub(crate) fn auto_advance(&self) -> bool {
        self.auto_advance
    }
    pub(crate) fn selection(&self) -> Selection {
        Selection {
            color: self.presets[self.active].colors[self.next],
            generation: self.generation,
        }
    }

    pub(crate) fn select_preset(&mut self, index: usize) -> bool {
        if index >= self.presets.len() || index == self.active {
            return false;
        }
        self.active = index;
        self.next = 0;
        self.changed();
        true
    }

    pub(crate) fn select_color(&mut self, index: usize) -> bool {
        if index >= self.presets[self.active].colors.len() || index == self.next {
            return false;
        }
        self.next = index;
        self.changed();
        true
    }

    pub(crate) fn cycle(&mut self, forward: bool) -> bool {
        let count = self.presets[self.active].colors.len();
        let next = if forward {
            (self.next + 1) % count
        } else {
            (self.next + count - 1) % count
        };
        self.select_color(next)
    }

    pub(crate) fn set_auto_advance(&mut self, value: bool) -> bool {
        if value == self.auto_advance {
            return false;
        }
        self.auto_advance = value;
        self.changed();
        true
    }

    pub(crate) fn replace_presets(
        &mut self,
        presets: Vec<Preset>,
        active: usize,
        next: usize,
    ) -> Result<bool, String> {
        validate(&presets, active, next)?;
        if self.presets == presets && self.active == active && self.next == next {
            return Ok(false);
        }
        self.presets = presets;
        self.active = active;
        self.next = next;
        self.changed();
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn reset(&mut self) -> bool {
        let defaults = Self::default();
        if self.presets == defaults.presets
            && self.active == 0
            && self.next == 0
            && self.auto_advance
        {
            return false;
        }
        self.presets = defaults.presets;
        self.active = 0;
        self.next = 0;
        self.auto_advance = true;
        self.changed();
        true
    }

    /// Consume intent only after an actual, successful new-cable transaction.
    /// An earlier gesture cannot advance a choice the user changed meanwhile.
    pub(crate) fn advance_on_commit(&mut self, selection: Selection) -> bool {
        self.auto_advance && self.selection() == selection && self.cycle(true)
    }

    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

fn validate(presets: &[Preset], active: usize, next: usize) -> Result<(), String> {
    if presets.is_empty() || presets.len() > MAX_PRESETS {
        return Err(format!("Use 1–{MAX_PRESETS} palettes."));
    }
    for (index, preset) in presets.iter().enumerate() {
        if preset.name.trim().is_empty() || preset.name.chars().count() > MAX_NAME_CHARS {
            return Err(format!("Palette names need 1–{MAX_NAME_CHARS} characters."));
        }
        if presets[..index]
            .iter()
            .any(|other| other.name.trim().to_lowercase() == preset.name.trim().to_lowercase())
        {
            return Err("Give each palette a different name.".into());
        }
        if preset.colors.is_empty() || preset.colors.len() > MAX_COLORS {
            return Err(format!("Use 1–{MAX_COLORS} colours in each palette."));
        }
    }
    if active >= presets.len() || next >= presets[active].colors.len() {
        return Err("Choose an existing palette and colour.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_roundtrip_preserves_editable_order_and_choice_without_persisting_tickets() {
        let mut palette = Palette::default();
        let mut presets = palette.presets().to_vec();
        presets[0].name = "My leads".into();
        presets[0].colors.swap(0, 3);
        presets[0].colors.remove(1);
        presets.push(Preset {
            name: "Mono".into(),
            colors: vec![[7, 8, 9]],
        });
        palette.replace_presets(presets, 0, 2).unwrap();
        palette.set_auto_advance(false);
        let restored = Palette::from_settings(&palette.settings());
        assert_eq!(restored.settings(), palette.settings());
        assert_eq!(restored.selection().generation, 0);
        assert!(!restored.auto_advance());
        assert_eq!(restored.presets()[0].name, "My leads");
    }

    #[test]
    fn malformed_preferences_fall_back_whole_palette_and_rejected_edits_preserve_choice() {
        let original = Palette::default();
        for broken in [
            Value::Null,
            json!({}),
            json!({"version":2}),
            {
                let mut v = original.settings();
                v["presets"][0]["colors"][0] = json!([256, 1, 2]);
                v
            },
            {
                let mut v = original.settings();
                v["next"] = json!(100);
                v
            },
            {
                let mut v = original.settings();
                v["presets"][0]["colors"] = json!([]);
                v
            },
            {
                let mut v = original.settings();
                v["presets"][1]["name"] = json!(" studio ");
                v
            },
        ] {
            assert_eq!(Palette::from_settings(&broken), original);
        }
        let mut palette = original.clone();
        assert!(palette.replace_presets(vec![], 0, 0).is_err());
        assert!(!palette.select_color(100));
        assert!(!palette.select_preset(100));
        assert_eq!(palette, original);
    }

    #[test]
    fn only_matching_success_ticket_advances_and_cycles_both_ways() {
        let mut palette = Palette::default();
        let captured = palette.selection();
        assert!(palette.advance_on_commit(captured));
        assert_eq!(palette.next_index(), 1);
        assert!(!palette.advance_on_commit(captured));
        assert!(palette.cycle(false));
        assert_eq!(palette.next_index(), 0);
        assert!(
            !palette.advance_on_commit(captured),
            "returning to same RGB must not revive old intent"
        );
        assert!(palette.cycle(false));
        assert_eq!(palette.next_index(), 5);
        let captured = palette.selection();
        palette.set_auto_advance(false);
        assert!(!palette.advance_on_commit(captured));
        let captured = palette.selection();
        assert!(!palette.advance_on_commit(captured));
        palette.reset();
        assert_eq!(palette.settings(), Palette::default().settings());
    }
}
