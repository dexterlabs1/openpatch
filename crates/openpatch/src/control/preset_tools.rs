use super::*;
use crate::presets;

impl Session {
    pub(super) fn capture_module_preset(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        let node = self
            .patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        let (running, source, frame) = if node.kind == "seq3" {
            if let Some(backend) = &self.live_module_events {
                let (running, frame) =
                    running::live_running(&backend.snapshot(), id, self.revision)?;
                (Some(running), "live", Some(frame))
            } else {
                (Some(running::initial_running(node)), "offline_reset", None)
            }
        } else {
            (None, "saved_parameters", None)
        };
        let preset = presets::capture(node, running)?;
        Ok(
            json!({"revision":self.revision,"id":id,"preset":preset,"state_source":source,"frame":frame}),
        )
    }

    pub(super) fn preset_file_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "save_module_preset" => {
                let filename = string(args, "filename")?;
                let path = self.file(filename, presets::EXTENSION)?;
                let preset = args.get("preset").ok_or("preset required")?;
                presets::save_new(&path, preset)?;
                Ok(json!({"filename":filename,"preset":preset}))
            }
            "read_module_preset" => {
                let filename = string(args, "filename")?;
                let preset = presets::read(&self.file(filename, presets::EXTENSION)?)?;
                Ok(json!({"filename":filename,"preset":preset}))
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

pub(super) fn schema() -> Value {
    json!({"type":"object","properties":{
        "format":{"const":"openpatch-module-preset"},
        "version":{"type":"integer","const":1},"kind":{"type":"string"},
        "params":{"type":"object","additionalProperties":{"type":"number"}},
        "bypassed":{"type":"boolean"}
    },"required":["format","version","kind","params"],"additionalProperties":false})
}
