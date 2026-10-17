//! Authored scene data. All edits/validation run on the control thread.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_FRAME_SCENE_KEYFRAMES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSceneKeyframe {
    pub id: u16,
    pub timestamp: u16,
    pub values: [u16; 4],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSceneData {
    #[serde(default)]
    pub next_id: u16,
    #[serde(default)]
    pub keyframes: Vec<FrameSceneKeyframe>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum FrameSceneEdit {
    Add {
        timestamp: u16,
        values: [u16; 4],
    },
    Update {
        keyframe_id: u16,
        #[serde(default)]
        timestamp: Option<u16>,
        #[serde(default)]
        values: Option<[u16; 4]>,
    },
    Remove {
        keyframe_id: u16,
    },
    Clear,
}

impl FrameSceneData {
    pub fn content_schema() -> serde_json::Value {
        use serde_json::json;
        json!({"type":"object","additionalProperties":false,
        "required":["type","keyframes","next_id"],"properties":{
            "type":{"const":"frame_scene"},
            "next_id":{"type":"integer","minimum":0,"maximum":65535,"default":0},
            "keyframes":{"type":"array","maxItems":64,"default":[],
                "description":"Strictly increasing timestamps and unique stable IDs. Values are source 16-bit gain controls, not voltages.",
                "items":{"type":"object","additionalProperties":false,"required":["id","timestamp","values"],
                    "properties":{"id":{"type":"integer","minimum":0,"maximum":65535},
                    "timestamp":{"type":"integer","minimum":0,"maximum":65535},
                    "values":{"type":"array","minItems":4,"maxItems":4,"items":{"type":"integer","minimum":0,"maximum":65535}}}}}
        }})
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.keyframes.len() > MAX_FRAME_SCENE_KEYFRAMES {
            return Err("Frame Scene supports at most 64 keyframes".into());
        }
        if self
            .keyframes
            .windows(2)
            .any(|w| w[0].timestamp >= w[1].timestamp)
        {
            return Err("Frame Scene timestamps must be strictly increasing".into());
        }
        let mut ids = BTreeSet::new();
        if self.keyframes.iter().any(|key| !ids.insert(key.id)) {
            return Err("Frame Scene keyframe IDs must be unique".into());
        }
        Ok(())
    }

    /// Atomic authoring edit. IDs survive movement/reordering and never alias an
    /// existing frame, including after the next-ID counter wraps.
    pub fn apply(&mut self, edit: FrameSceneEdit) -> Result<Option<u16>, String> {
        self.validate()?;
        let mut candidate = self.clone();
        let id = match edit {
            FrameSceneEdit::Add { timestamp, values } => {
                if candidate.keyframes.len() == MAX_FRAME_SCENE_KEYFRAMES {
                    return Err("Frame Scene is full; update or remove an existing frame".into());
                }
                while candidate
                    .keyframes
                    .iter()
                    .any(|k| k.id == candidate.next_id)
                {
                    candidate.next_id = candidate.next_id.wrapping_add(1);
                }
                let id = candidate.next_id;
                candidate.next_id = id.wrapping_add(1);
                candidate.keyframes.push(FrameSceneKeyframe {
                    id,
                    timestamp,
                    values,
                });
                Some(id)
            }
            FrameSceneEdit::Update {
                keyframe_id,
                timestamp,
                values,
            } => {
                if timestamp.is_none() && values.is_none() {
                    return Err("Frame Scene update requires timestamp or values".into());
                }
                let key = candidate
                    .keyframes
                    .iter_mut()
                    .find(|k| k.id == keyframe_id)
                    .ok_or("Frame Scene keyframe ID does not exist")?;
                if let Some(timestamp) = timestamp {
                    key.timestamp = timestamp;
                }
                if let Some(values) = values {
                    key.values = values;
                }
                Some(keyframe_id)
            }
            FrameSceneEdit::Remove { keyframe_id } => {
                let index = candidate
                    .keyframes
                    .iter()
                    .position(|k| k.id == keyframe_id)
                    .ok_or("Frame Scene keyframe ID does not exist")?;
                candidate.keyframes.remove(index);
                Some(keyframe_id)
            }
            FrameSceneEdit::Clear => {
                candidate.keyframes.clear();
                None
            }
        };
        candidate.keyframes.sort_by_key(|key| key.timestamp);
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }
}

impl FrameSceneEdit {
    pub fn schema() -> serde_json::Value {
        use serde_json::json;
        let word = json!({"type":"integer","minimum":0,"maximum":65535});
        let values = json!({"type":"array","minItems":4,"maxItems":4,"items":word});
        json!({"oneOf":[
            {"type":"object","additionalProperties":false,"required":["operation","timestamp","values"],
                "properties":{"operation":{"const":"add"},"timestamp":word,"values":values}},
            {"type":"object","additionalProperties":false,"required":["operation","keyframe_id"],
                "anyOf":[{"required":["timestamp"]},{"required":["values"]}],
                "properties":{"operation":{"const":"update"},"keyframe_id":word,"timestamp":word,"values":values}},
            {"type":"object","additionalProperties":false,"required":["operation","keyframe_id"],
                "properties":{"operation":{"const":"remove"},"keyframe_id":word}},
            {"type":"object","additionalProperties":false,"required":["operation"],"properties":{"operation":{"const":"clear"}}}
        ]})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_scene_ids_survive_reordering_and_collisions_reject_atomically() {
        let mut data = FrameSceneData::default();
        let a = data
            .apply(FrameSceneEdit::Add {
                timestamp: 100,
                values: [1; 4],
            })
            .unwrap()
            .unwrap();
        let b = data
            .apply(FrameSceneEdit::Add {
                timestamp: 200,
                values: [2; 4],
            })
            .unwrap()
            .unwrap();
        data.apply(FrameSceneEdit::Update {
            keyframe_id: a,
            timestamp: Some(300),
            values: None,
        })
        .unwrap();
        assert_eq!(
            data.keyframes.iter().map(|k| k.id).collect::<Vec<_>>(),
            [b, a]
        );
        let before = data.clone();
        for edit in [
            FrameSceneEdit::Update {
                keyframe_id: a,
                timestamp: Some(200),
                values: None,
            },
            FrameSceneEdit::Remove { keyframe_id: 400 },
            FrameSceneEdit::Add {
                timestamp: 200,
                values: [3; 4],
            },
        ] {
            assert!(data.apply(edit).is_err());
            assert_eq!(data, before);
        }
    }

    #[test]
    fn frame_scene_capacity_allows_update_and_ids_wrap_without_aliasing() {
        let mut data = FrameSceneData {
            next_id: u16::MAX,
            ..Default::default()
        };
        for timestamp in 0..64 {
            data.apply(FrameSceneEdit::Add {
                timestamp,
                values: [timestamp; 4],
            })
            .unwrap();
        }
        assert_eq!(data.keyframes[0].id, u16::MAX);
        assert_eq!(data.keyframes[1].id, 0);
        let before = data.clone();
        assert!(data
            .apply(FrameSceneEdit::Add {
                timestamp: 100,
                values: [0; 4]
            })
            .is_err());
        assert_eq!(data, before);
        data.apply(FrameSceneEdit::Update {
            keyframe_id: 0,
            timestamp: None,
            values: Some([65535; 4]),
        })
        .unwrap();
        assert_eq!(data.keyframes[1].values, [65535; 4]);
        let json = serde_json::to_string(&data).unwrap();
        assert_eq!(serde_json::from_str::<FrameSceneData>(&json).unwrap(), data);
    }
}
