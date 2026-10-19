//! Runtime recording observes the callback; durable scale changes use the
//! same revision-fenced edit transaction as manual edits and MCP content.
use super::*;

impl Session {
    pub(super) fn recall_random_capture_tool(&mut self, args: &Value) -> Result<Value, String> {
        let id = string(args, "id")?;
        let action = string(args, "action")?;
        if !matches!(action, "begin" | "stop" | "cancel" | "inspect" | "commit") {
            return Err("Invalid Recall Random capture action".into());
        }
        if args.as_object().is_some_and(|a| {
            a.keys()
                .any(|k| !["id", "action", "slot", "expected_revision"].contains(&k.as_str()))
        }) {
            return Err("Unknown Recall Random capture argument".into());
        }

        self.check_revision(expected_revision(args)?)?;
        let node = self
            .patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        if node.kind != "recall_random" {
            return Err("capture requires a Recall Random module".into());
        }
        let slot = args.get("slot").map_or(Ok(0), |v| {
            v.as_u64()
                .filter(|n| *n < 6)
                .map(|n| n as u8)
                .ok_or("slot must be 0..5")
        })?;
        let live = self
            .live_module_events
            .as_ref()
            .ok_or("Recall Random capture requires a live audio backend")?;
        if action == "begin" {
            self.require_document_execution()?;
        }
        let receipt = live.recall_random_capture(
            id,
            if action == "commit" {
                "inspect"
            } else {
                action
            },
            slot,
            self.revision,
        )?;
        if action != "commit" {
            return Ok(receipt);
        }
        let state: openpatch_core::RecallRandomCaptureState = serde_json::from_value(
            receipt
                .get("capture")
                .cloned()
                .ok_or("No Recall Random capture is available")?,
        )
        .map_err(|error| format!("Invalid capture receipt: {error}"))?;
        if state.status != openpatch_core::RecallRandomCaptureStatus::Complete {
            return Err("Capture is not complete; stop recording and poll its status first".into());
        }
        if state.revision != self.revision {
            return Err("Capture is stale because the document changed; record again or copy the observed scale manually".into());
        }
        if state.slot >= 6 {
            return Err("Invalid capture slot".into());
        }
        if receipt["id"].as_str() != Some(id) {
            return Err("Capture module identity mismatch".into());
        }
        let instance = receipt["instance"]
            .as_u64()
            .ok_or("Capture instance identity is missing")?;
        let key = (
            self.live_backend_epoch,
            id.to_owned(),
            instance,
            state.revision,
            state.capture_id,
        );
        if self.recall_captures_committed.contains(&key) {
            return Err("This capture has already been committed".into());
        }
        let scale = state.scale.to_scale();
        scale.validate()?;
        let mut data = match &node.content {
            Some(openpatch_core::ModuleContent::RecallRandom(data)) => data.clone(),
            None => Box::new(openpatch_core::RecallRandomData::default()),
            _ => return Err("Recall Random has mismatched content".into()),
        };
        data.scales[usize::from(state.slot)] = scale;
        let mut result = self.execute("set_module_content", json!({"id":id,"content":openpatch_core::ModuleContent::RecallRandom(data),"expected_revision":state.revision}))?;
        self.recall_captures_committed
            .retain(|key| key.3 == self.revision);
        self.recall_captures_committed.insert(key);
        result["capture_committed"] =
            json!({"id":id,"slot":state.slot,"capture_id":state.capture_id});
        Ok(result)
    }
}
