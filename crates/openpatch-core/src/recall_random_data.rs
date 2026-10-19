//! Authored weighted scales, separate from transient source loops and recorder state.
use serde::{Deserialize, Serialize};

pub const RECALL_RANDOM_SCALE_SLOTS: usize = 6;
pub const MAX_RECALL_RANDOM_DEGREES: usize = 16;
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallRandomDegree {
    pub voltage: f32,
    pub weight: u8,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallRandomScale {
    pub base_interval: f32,
    pub degrees: Vec<RecallRandomDegree>,
}
impl RecallRandomScale {
    pub fn validate(&self) -> Result<(), String> {
        if !self.base_interval.is_finite() || self.base_interval <= 0.0 {
            return Err("Recall Random scale interval must be positive and finite".into());
        }
        if self.degrees.is_empty() || self.degrees.len() > MAX_RECALL_RANDOM_DEGREES {
            return Err("Recall Random scale requires 1..=16 degrees".into());
        }
        if self.degrees.iter().any(|d| !d.voltage.is_finite())
            || self
                .degrees
                .windows(2)
                .any(|w| w[0].voltage >= w[1].voltage)
        {
            return Err("Recall Random degrees must be finite and strictly increasing (negative recorder degrees are allowed)".into());
        }
        if !self.degrees.iter().any(|d| d.weight == 255) {
            return Err("Recall Random scale requires at least one weight 255".into());
        }
        Ok(())
    }
    pub(crate) fn kernel(&self) -> crate::recall_random::kernel::quantizer::Scale {
        use crate::recall_random::kernel::quantizer::{Degree, Scale};
        let mut scale = Scale {
            base_interval: self.base_interval,
            num_degrees: self.degrees.len(),
            degree: [Degree::default(); 16],
        };
        for (out, d) in scale.degree.iter_mut().zip(&self.degrees) {
            *out = Degree {
                voltage: d.voltage,
                weight: d.weight,
            };
        }
        scale
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecallRandomData {
    pub scales: [RecallRandomScale; 6],
    pub active_slot: u8,
}
impl Default for RecallRandomData {
    fn default() -> Self {
        Self {
            active_slot: 0,
            scales: crate::recall_random::default_scales().map(|scale| RecallRandomScale {
                base_interval: scale.base_interval,
                degrees: scale.degree[..scale.num_degrees]
                    .iter()
                    .map(|d| RecallRandomDegree {
                        voltage: d.voltage,
                        weight: d.weight,
                    })
                    .collect(),
            }),
        }
    }
}
impl RecallRandomData {
    pub fn validate(&self) -> Result<(), String> {
        if self.active_slot >= 6 {
            return Err("Recall Random active slot must be 0..=5".into());
        }
        for scale in &self.scales {
            scale.validate()?;
        }
        Ok(())
    }
    pub fn content_schema() -> serde_json::Value {
        serde_json::json!({"type":"object","additionalProperties":false,"required":["type","scales","active_slot"],"properties":{
            "type":{"const":"recall_random"},"active_slot":{"type":"integer","minimum":0,"maximum":5,"default":0},
            "scales":{"type":"array","minItems":6,"maxItems":6,"items":{"type":"object","additionalProperties":false,"required":["base_interval","degrees"],"properties":{
                "base_interval":{"type":"number","exclusiveMinimum":0},"degrees":{"type":"array","minItems":1,"maxItems":16,"description":"Strictly increasing finite voltages, including negative recorder degrees; at least one byte weight must be 255.","items":{"type":"object","additionalProperties":false,"required":["voltage","weight"],"properties":{"voltage":{"type":"number"},"weight":{"type":"integer","minimum":0,"maximum":255}}}}
            }}}
        }})
    }
}
/// Fixed callback-safe scale publication. Convert to authored Vec data off callback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecallRandomCapturedScale {
    pub base_interval: f32,
    pub num_degrees: u8,
    pub degrees: [RecallRandomDegree; 16],
}
impl RecallRandomCapturedScale {
    pub fn to_scale(&self) -> RecallRandomScale {
        RecallRandomScale {
            base_interval: self.base_interval,
            degrees: self.degrees[..usize::from(self.num_degrees).min(16)].to_vec(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallRandomCaptureStatus {
    Recording,
    Complete,
    Empty,
    Cancelled,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecallRandomCaptureState {
    pub capture_id: u64,
    pub revision: u64,
    pub slot: u8,
    pub status: RecallRandomCaptureStatus,
    pub scale: RecallRandomCapturedScale,
}
