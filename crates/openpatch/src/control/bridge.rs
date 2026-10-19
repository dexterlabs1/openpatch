//! Native transport observation. No listener/secret/lease is stored in a patch.
use super::*;

#[derive(Clone, Debug)]
struct Transport {
    generation: u64,
    state: &'static str,
    attachment_id: Option<String>,
    admitted_clients: usize,
    stdio_enabled: bool,
    last_error: Option<String>,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            generation: 0,
            state: "stopped",
            attachment_id: None,
            admitted_clients: 0,
            stdio_enabled: false,
            last_error: None,
        }
    }
}
#[derive(Default)]
pub struct BridgeObservation(Mutex<Transport>);
impl BridgeObservation {
    pub fn set_stdio_enabled(&self, enabled: bool) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stdio_enabled = enabled;
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn reserve_start(&self) -> Result<u64, String> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| "bridge observation unavailable")?;
        if matches!(state.state, "starting" | "running") {
            return Err("this session already has a local MCP bridge".into());
        }
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or("bridge generation exhausted")?;
        state.state = "starting";
        state.attachment_id = None;
        state.admitted_clients = 0;
        state.last_error = None;
        Ok(state.generation)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn running(&self, generation: u64, id: &str) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation && state.state == "starting" {
            state.state = "running";
            state.attachment_id = Some(id.into());
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn clients(&self, generation: u64, count: usize) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation && state.state == "running" {
            state.admitted_clients = count;
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn failed(&self, generation: u64, error: String) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation && matches!(state.state, "starting" | "running") {
            state.state = "failed";
            state.attachment_id = None;
            state.admitted_clients = 0;
            state.last_error = Some(error.chars().take(256).collect());
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn stopped(&self, generation: u64) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.generation == generation {
            state.state = "stopped";
            state.attachment_id = None;
            state.admitted_clients = 0;
            state.last_error = None;
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn is_running(&self, generation: u64) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        state.generation == generation && state.state == "running"
    }
    pub fn snapshot(&self) -> Value {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        json!({"generation":state.generation,"state":state.state,
            "attachment_id":state.attachment_id,"admitted_clients":state.admitted_clients,
            "stdio_enabled":state.stdio_enabled,"last_error":state.last_error})
    }
}

impl Session {
    pub fn bridge_observation(&self) -> Arc<BridgeObservation> {
        self.bridge_observation.clone()
    }
    pub fn bridge_status(&self) -> Value {
        json!({"revision":self.revision,"document_generation":self.project_save.generation(),
            "transport":self.bridge_observation.snapshot(),
            "authority":{"kind":"same_user_shared_mutation","writer_lease":false,"stdio_client_count":null},
            "save":self.project_save_status(),"last_operation":self.last_operation,
            "bridge_panels":self.patch.nodes.iter().filter(|node| node.kind=="rackmcp_bridge").count(),
            "compatibility":"native host adaptation; no Rack pairing secret, HMAC, TCP port or writer lease"})
    }
    pub(super) fn inspect_bridge_tool(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        if args.get("id").is_some() {
            let id = string(args, "id")?;
            if !self
                .patch
                .nodes
                .iter()
                .any(|node| node.id == id && node.kind == "rackmcp_bridge")
            {
                return Err("module is not a Bridge".into());
            }
        }
        Ok(self.bridge_status())
    }
    /// Native host lifecycle actions are not transport-callable patch tools.
    pub fn record_bridge_action(&mut self, name: &str, success: bool) {
        self.last_operation = Some(
            json!({"name":format!("bridge.{name}"),"origin":"native_host",
            "outcome":if success {"completed"} else {"failed"},"revision":self.revision}),
        );
    }
    pub(super) fn record_operation(
        &mut self,
        name: &str,
        local: bool,
        result: &Result<Value, String>,
    ) {
        let meaningful = EDIT_TOOLS.contains(&name)
            || matches!(
                name,
                "rackmcp_transport"
                    | "undo"
                    | "redo"
                    | "trigger_module"
                    | "replace_patch"
                    | "replace_document"
            )
            || [
                "save_", "load_", "import_", "start_", "pause_", "resume_", "cancel_", "set_",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        if !meaningful {
            return;
        }
        let accepted = matches!(
            name,
            "rackmcp_transport"
                | "start_listen_capture"
                | "cancel_listen_capture"
                | "start_construction"
        );
        self.last_operation = Some(json!({"name":name.chars().take(64).collect::<String>(),
            "origin":if local {"local_attach"} else {"shared_session"},
            "outcome":if result.is_err(){"failed"}else if accepted{"accepted"}else{"completed"},"revision":self.revision}));
    }
}
