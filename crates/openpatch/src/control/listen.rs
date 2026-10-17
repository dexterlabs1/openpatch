//! Listen observes the existing live graph; no patch edit or offline fallback.
use super::*;

impl Session {
    fn listen_node(&self, id: &str) -> Result<&Node, String> {
        let node = self
            .patch
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        if node.kind != "rackmcp_listen" {
            return Err("module is not a Listen".into());
        }
        Ok(node)
    }
    pub(super) fn start_listen_capture_tool(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = if args.get("id").is_some() {
            let id = string(args, "id")?;
            self.listen_node(id)?;
            id
        } else {
            let mut modules = self
                .patch
                .nodes
                .iter()
                .filter(|node| node.kind == "rackmcp_listen");
            let node = modules.next().ok_or("no Listen module in patch")?;
            if modules.next().is_some() {
                return Err("multiple Listen modules; specify id".into());
            }
            &node.id
        };
        let seconds = args["seconds"]
            .as_f64()
            .filter(|value| value.is_finite() && (0.1..=30.0).contains(value))
            .ok_or("seconds must be finite in 0.1..30")?;
        let write_wav = args
            .get("write_wav")
            .map(|value| value.as_bool().ok_or("write_wav must be a boolean"))
            .transpose()?
            .unwrap_or(false);
        let backend = self
            .live_module_events
            .as_ref()
            .ok_or("Listen capture unsupported: no live audio backend attached")?;
        let capture = backend.start_listen_capture(id, seconds, write_wav, &self.workspace)?;
        Ok(json!({"revision":self.revision,"id":id,"capture":capture}))
    }
    pub(super) fn listen_capture_job_tool(
        &self,
        name: &str,
        args: &Value,
    ) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let job = string(args, "job")?;
        let backend = self
            .live_module_events
            .as_ref()
            .ok_or("Listen capture unsupported: no live audio backend attached")?;
        let capture = if name == "cancel_listen_capture" {
            backend.cancel_listen_capture(job)?
        } else {
            backend.inspect_listen_capture(job)?
        };
        Ok(json!({"revision":self.revision,"capture":capture}))
    }
    pub(super) fn inspect_listen_tool(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let id = string(args, "id")?;
        self.listen_node(id)?;
        let unavailable = |reason: &str| json!({"revision":self.revision,"id":id,"available":false,"reason":reason});
        if !self.execution_disposition().is_runnable() {
            return Ok(unavailable("document execution is suspended"));
        }
        let Some(backend) = &self.live_module_events else {
            return Ok(unavailable("no live audio backend attached"));
        };
        let snapshot = backend.snapshot();
        let module = &snapshot["modules"][id];
        if snapshot["coherent"] != true
            || module["available"] != true
            || module["revision"].as_u64() != Some(self.revision)
            || module["kind"] != "rackmcp_listen"
            || !module["listen"].is_object()
        {
            return Ok(unavailable("current Listen status has not been published"));
        }
        Ok(
            json!({"revision":self.revision,"id":id,"available":true,"frame":module["frame"],"listen":module["listen"],"capture":backend.latest_listen_capture(id)}),
        )
    }
}
