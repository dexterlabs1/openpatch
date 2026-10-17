//! Live source transport is a performance request, not a sequence-data edit.
use super::*;
use openpatch_core::RackMcpTransport;

impl Session {
    pub(super) fn rackmcp_transport_tool(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        let node = self.rackmcp_node(id)?;
        let state = node.rackmcp_sequence.clone().unwrap_or_default();
        let action = string(args, "action")?;
        let pattern = args.get("pattern").and_then(Value::as_str);
        if args.get("quantize").is_some() && pattern.is_none() {
            return Err("quantize requires a pattern".into());
        }
        if action == "seek" && pattern.is_some() {
            return Err("seek does not accept a pattern".into());
        }
        if action != "seek" && args.get("bar").is_some() {
            return Err("bar is only valid for seek".into());
        }
        let mut requests = Vec::with_capacity(2);
        if let Some(name) = pattern {
            let index = state
                .data
                .patterns
                .iter()
                .position(|p| p.name == name)
                .ok_or("pattern not found")?;
            requests.push(RackMcpTransport::Select {
                pattern: index as u8,
                now: args["quantize"] == "now",
            });
        }
        let request = match action {
            "play" => Some(RackMcpTransport::Play),
            "stop" => Some(RackMcpTransport::Stop),
            "toggle" => Some(RackMcpTransport::Toggle),
            "reset" => Some(RackMcpTransport::Reset),
            "select" if pattern.is_some() => None,
            "select" => return Err("select requires a pattern".into()),
            "seek" => {
                let bar = args["bar"]
                    .as_f64()
                    .filter(|bar| bar.is_finite() && (0.0..=100000.0).contains(bar))
                    .ok_or("seek requires a finite bar in 0..100000")?;
                Some(RackMcpTransport::Seek { bar })
            }
            _ => return Err("unsupported RackMCP transport action".into()),
        };
        requests.extend(request);
        let backend = self
            .live_module_events
            .as_ref()
            .ok_or("RackMCP transport unsupported: no live audio backend attached")?;
        let receipt = backend.rackmcp_transport(id, &requests, &state.data)?;
        Ok(json!({"revision":self.revision,"receipt":receipt}))
    }

    fn rackmcp_node(&self, id: &str) -> Result<&Node, String> {
        let node = self
            .patch
            .nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or("module not found")?;
        if node.kind != "rackmcp_sequencer" {
            return Err("module is not a RackMCP Sequencer".into());
        }
        Ok(node)
    }

    pub(super) fn inspect_rackmcp_sequence_tool(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        self.rackmcp_node(id)?;
        let unavailable = |reason: &str| {
            json!({"revision":self.revision,"id":id,
            "available":false,"reason":reason})
        };
        if !self.execution_disposition().is_runnable() {
            return Ok(unavailable("document execution is suspended"));
        }
        let Some(backend) = &self.live_module_events else {
            return Ok(unavailable("no live audio backend attached"));
        };
        let snapshot = backend.snapshot();
        let module = &snapshot["modules"][id];
        if snapshot["coherent"].as_bool() == Some(false)
            || module["available"] != true
            || module["revision"].as_u64() != Some(self.revision)
            || module["kind"] != "rackmcp_sequencer"
            || !module["rackmcp_sequence"].is_object()
        {
            return Ok(unavailable(
                "current sequence status has not been published",
            ));
        }
        Ok(json!({"revision":self.revision,"id":id,"available":true,
            "frame":module["frame"],"rackmcp_sequence":module["rackmcp_sequence"]}))
    }
}

pub(super) fn validate_selection(node: &Node, state: SavedModuleState) -> Result<(), String> {
    if let SavedModuleState::RackMcpSelection(name) = state {
        let valid = node.rackmcp_sequence.as_ref().map_or_else(
            || SavedModuleState::from_node(node) == Some(state),
            |state| {
                state
                    .data
                    .patterns
                    .iter()
                    .any(|pattern| pattern.name == name.as_str())
            },
        );
        if !valid {
            return Err(format!(
                "live selected pattern is not in module '{}'",
                node.id
            ));
        }
    }
    Ok(())
}
