use super::*;
use crate::presets;

impl Session {
    pub(super) fn capture_module_preset(&self, args: &Value) -> Result<Value, String> {
        self.check_midi_cc_capture()?;
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        let node = self
            .patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        let backend = self
            .live_module_events
            .as_ref()
            .filter(|_| self.execution_disposition().is_runnable());
        let snapshot = backend.map(|backend| backend.snapshot());
        let mut frame = None;
        let mut captured = node.clone();
        let mut source = if self.execution_disposition().is_runnable() {
            "saved_parameters"
        } else {
            "suspended_persisted"
        };
        let state = if let Some(initial) = SavedModuleState::from_node(node) {
            if let Some(snapshot) = &snapshot {
                let (value, seen) = running::live_saved(snapshot, id, self.revision, initial)?;
                frame = Some(seen);
                source = "live";
                Some(value)
            } else {
                if self.execution_disposition().is_runnable() {
                    source = "offline_reset";
                }
                Some(initial)
            }
        } else {
            None
        };
        if let Some(state) = state {
            super::rackmcp_transport::validate_selection(node, state)?;
        }
        if backend.is_some_and(|backend| backend.supports_live_parameters()) {
            for &name in openpatch_core::runtime_parameter_names(&node.kind) {
                let value =
                    super::parameters::observe(self, node, name, snapshot.as_ref(), &mut frame)?;
                captured.params.insert(name.into(), value);
                source = "live";
            }
        }
        self.check_midi_cc_capture()?;
        let preset = presets::capture_state_with_assets(&captured, state, &self.patch.assets)?;
        let mut reply = preset_reply(&preset, args)?;
        reply["revision"] = json!(self.revision);
        reply["id"] = json!(id);
        reply["state_source"] = json!(source);
        reply["frame"] = json!(frame);
        Ok(reply)
    }

    pub(super) fn preset_file_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "save_module_preset" => {
                let filename = string(args, "filename")?;
                let path = self.file(filename, presets::EXTENSION)?;
                let preset = args.get("preset").ok_or("preset required")?;
                let mut reply = preset_reply(preset, args)?;
                presets::save_new(&path, preset)?;
                reply["filename"] = json!(filename);
                Ok(reply)
            }
            "read_module_preset" => {
                let filename = string(args, "filename")?;
                let (preset, digest) =
                    presets::read_with_digest(&self.file(filename, presets::EXTENSION)?)?;
                let mut reply = preset_reply(&preset, args)?;
                reply["filename"] = json!(filename);
                if matches!(
                    preset["kind"].as_str(),
                    Some("wavetable_vco" | "wavetable_lfo")
                ) {
                    reply["file_sha256"] = json!(digest);
                }
                Ok(reply)
            }
            "list_module_presets" => {
                let kind = args.get("kind").map(|_| string(args, "kind")).transpose()?;
                let offset = args.get("offset").map_or(Ok(0), |value| {
                    value
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or("offset must be a nonnegative integer")
                })?;
                let limit = args.get("limit").map_or(Ok(50), |value| {
                    value
                        .as_u64()
                        .filter(|value| (1..=100).contains(value))
                        .map(|value| value as usize)
                        .ok_or("limit must be an integer in 1..100")
                })?;
                let query = optional_string(args, "query", "")?;
                if query.len() > 120
                    || !query.is_ascii()
                    || query.bytes().any(|byte| byte.is_ascii_control())
                {
                    return Err("query must contain at most 120 printable ASCII characters".into());
                }
                presets::list(&self.workspace, kind, query, offset, limit)
            }
            _ => unreachable!(),
        }
    }
}

fn preset_reply(preset: &Value, args: &Value) -> Result<Value, String> {
    let include_data = args
        .get("include_asset_data")
        .map(|value| {
            value
                .as_bool()
                .ok_or("include_asset_data must be a boolean")
        })
        .transpose()?
        .unwrap_or(false);
    if !include_data
        && matches!(
            preset["kind"].as_str(),
            Some("wavetable_vco" | "wavetable_lfo")
        )
    {
        Ok(json!({"preset_summary":presets::summary(preset)?}))
    } else {
        Ok(json!({"preset":preset}))
    }
}

pub(super) fn schema() -> Value {
    json!({"type":"object","properties":{
        "format":{"const":"openpatch-module-preset"},
        "version":{"type":"integer","const":1},"kind":{"type":"string"},
        "params":{"type":"object","additionalProperties":{"type":"number"}},
        "bypassed":{"type":"boolean"},
        "score":{"type":"object"},
        "rackmcp_sequence":{"type":"object"},
        "content":content_schema(),
        "assets":{"type":"object","maxProperties":1,"additionalProperties":{
            "type":"object","properties":{
                "encoding":{"const":"f32le-base64-v1"},"data":{"type":"string","maxLength":2796204}
            },"required":["encoding","data"],"additionalProperties":false
        }}
    },"required":["format","version","kind","params"],"additionalProperties":false})
}

pub(super) fn content_schema() -> Value {
    json!({"oneOf":[
        openpatch_core::FrameSceneData::content_schema(),
        {"type":"object","properties":{
            "type":{"const":"notes"},
            "text":{"type":"string","maxLength":8192,"description":"Multiline Unicode text; at most 8192 UTF-8 bytes."}
        },"required":["type","text"],"additionalProperties":false},
        {"type":"object","properties":{
            "type":{"const":"blank"},
            "width_hp":{"type":"integer","minimum":3,"maximum":96}
        },"required":["type","width_hp"],"additionalProperties":false},
        {"type":"object","properties":{
            "type":{"const":"wavetable"},
            "asset_id":{"type":"string","pattern":"^[0-9a-f]{64}$"},
            "wave_points":{"type":"integer","minimum":2,"maximum":524288},
            "name":{"type":"string","maxLength":1024}
        },"required":["type","asset_id","wave_points","name"],"additionalProperties":false}
    ]})
}
