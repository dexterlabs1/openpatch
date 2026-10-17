//! Original MIT OpenPatch host-panel model, informed by the pinned RackMCP
//! Bridge 2.0.0 contract (Zachary Scheffler, GPL-3.0-or-later, revision
//! 285e84c5ff2d3ecef25890da07f578af6a0c5ca6). No source DSP equations or
//! implementation are copied. See docs/RACKMCP_CONTROL_CONTRACT.md.
use crate::Node;
use serde_json::{json, Value};

pub(crate) fn catalog() -> Value {
    json!({"kind":"rackmcp_bridge","title":"RACKMCP BRIDGE","model":"host-only",
        "host_only":true,
        "description":"Native host session, saved-document and control service status. App actions manage the actual native listener independently of panel presence.",
        "inputs":[],"outputs":[],"params":{},"hp":6,"tags":["Utility"],
        "source":{"plugin":"RackMCP","version":"2.0.0","revision":"285e84c5ff2d3ecef25890da07f578af6a0c5ca6","license":"GPL-3.0-or-later"},
        "mapping_fidelity":"MusicalRole"})
}

pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"] = json!({"version_1":"unsupported","version_2":"host-only; no signal ports or DSP state","maximum_channels":0});
    module["input_names"] = json!([]);
    module["output_names"] = json!([]);
    module["light_names"] = json!([]);
    module["host_status_policy"] = json!({"observation_ms":100,"source_lamps_frames":512,"authority":"actual shared native host session and authored-content/durable-state save baseline","adaptation":"native host UI observation replaces source DSP lamp cadence; no Rack SDK, transport, DSP or hardware equality claim"});
    module["state_adoption"] = json!("no module runtime or saved state; creation, duplication, import, initialize, undo, removal and reopen do not start or restart services or bind audio hardware");
    module["control_policy"] = json!("listener start/stop/restart are explicit app actions; no signal parameters, source reset-pairing alias, pairing secret, endpoint, client or writer lease is stored in this module");
}

pub(crate) fn validate(node: &Node) -> Result<(), String> {
    if let Some(key) = node.params.keys().next() {
        return Err(format!("node '{}': unknown parameter '{key}'", node.id));
    }
    Ok(())
}
