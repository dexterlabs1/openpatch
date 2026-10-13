//! Bounded newline-delimited JSON-RPC over stdio. No network listener or model API.
use crate::control::{execute_shared, tools, Application, SharedSession};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const MAX_LINE: usize = 1_048_576;
const VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

// Avoid unbounded read_line allocation on hostile/malformed client input.
fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Result<Vec<u8>, ()>>> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return if line.is_empty() && !oversized {
                Ok(None)
            } else {
                Ok(Some(if oversized { Err(()) } else { Ok(line) }))
            };
        }
        let n = buf
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(buf.len());
        let done = buf[n - 1] == b'\n';
        if !oversized {
            if line.len() + n > MAX_LINE {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(&buf[..n]);
            }
        }
        reader.consume(n);
        if done {
            return Ok(Some(if oversized { Err(()) } else { Ok(line) }));
        }
    }
}

struct Connection {
    session: SharedSession,
    live_desktop: bool,
    initialized: bool,
    ready: bool,
}
impl Connection {
    fn handle(&mut self, msg: Value) -> Option<Value> {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let valid_id = msg
            .get("id")
            .is_none_or(|v| v.is_string() || v.is_i64() || v.is_u64());
        if !msg.is_object() || msg["jsonrpc"] != "2.0" || !msg["method"].is_string() || !valid_id {
            return Some(error(Value::Null, -32600, "invalid JSON-RPC request"));
        }
        let method = msg["method"].as_str().unwrap();
        if msg.get("id").is_none() {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        }
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        if !params.is_object() {
            return Some(error(id, -32602, "params must be an object"));
        }
        let result = match method {
            "initialize" => {
                if self.initialized {
                    return Some(error(id, -32600, "already initialized"));
                }
                let requested = match params.get("protocolVersion").and_then(Value::as_str) {
                    Some(v) => v,
                    None => return Some(error(id, -32602, "protocolVersion required")),
                };
                self.initialized = true;
                let version = if VERSIONS.contains(&requested) {
                    requested
                } else {
                    VERSIONS[0]
                };
                let instructions = if self.live_desktop {
                    "OpenPatch is a native modular synthesizer. This connection controls the same live patch shown in the desktop window. Start with catalog and inspect_patch. Edits update the window and its audio engine; speaker playback is controlled in the desktop window. Use expected_revision to avoid overwriting concurrent GUI edits. New patches use engine v2 with named outputs, up to 16 channels, and one sample per cable; inspect the patch version. analyze_signal measures any named output offline without changing the patch. capture_signal returns bounded peak-preserving waveform bins and calibrated channel statistics offline from reset; it is not a live scope. audio_stereo selects explicit L/R playback and WAV export; other outputs use the first channel. poly_sum mixes voices. render_audio, analyze_signal, and capture_signal accept sorted sample-frame midi_events and module_events to reproduce performances from reset without live playback or history edits. trigger_module queues live seq3, logic, or process actions; its receipt confirms acceptance only. inspect_performance reports actual published module state, controls, lights, and callback counters. rotate_sequence commits one undoable column rotation. initialize_modules restores eligible defaults with audited per-kind initialization; randomize_modules requires a u32 seed and follows catalog eligibility/distributions. Both support atomic batches and preview; undo/redo restore saved controls and captured SEQ3 Run. capture_module_preset snapshots effective controls/current Run; save_module_preset writes exactly reviewed contents to a new .opreset file. Browse with list_module_presets/read_module_preset and apply reviewed inline contents with apply_module_preset; loading does not Initialize or replace the module. Filenames are confined to the configured workspace; existing files are never overwritten. No VCV plugin compatibility."
                } else {
                    "OpenPatch is an offline modular synthesizer. Start with catalog and inspect_patch. New patches use engine v2 with named outputs and polyphonic cables. Build a patch, analyze_signal or capture_signal any output, then render_audio. capture_signal returns calibrated min/max waveform bins offline from reset, not live audio. audio_stereo selects stereo WAV export; other outputs use the first channel. Add poly_sum for a mono mix. Optional sorted sample-frame midi_events and module_events drive reproducible performances during render_audio, analyze_signal, and capture_signal without modifying the patch. trigger_module is unsupported without a live audio backend; inspect_performance reports that boundary. rotate_sequence commits one undoable seq3 column rotation. initialize_modules restores eligible defaults with audited per-kind initialization; randomize_modules requires a u32 seed and follows catalog eligibility/distributions. Both support atomic batches and preview; undo/redo restore saved controls and captured SEQ3 Run. capture_module_preset snapshots effective controls/current Run; save_module_preset writes exactly reviewed contents to a new .opreset file. Browse with list_module_presets/read_module_preset and apply reviewed inline contents with apply_module_preset; loading does not Initialize or replace the module. Filenames are confined to the configured workspace; existing files are never overwritten. This headless process owns an independent patch. No speaker playback or VCV plugin compatibility."
                };
                json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"openpatch","version":env!("CARGO_PKG_VERSION")},"instructions":instructions})
            }
            "ping" => json!({}),
            _ if !self.ready => {
                return Some(error(
                    id,
                    -32002,
                    "initialize and notifications/initialized required",
                ))
            }
            "tools/list" => json!({"tools":tools()}),
            "tools/call" => {
                let name = match params.get("name").and_then(Value::as_str) {
                    Some(n) => n,
                    None => return Some(error(id, -32602, "tool name required")),
                };
                if !tools()
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|t| t["name"] == name)
                {
                    return Some(error(id, -32602, "unknown tool"));
                }
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match execute_shared(&self.session, name, args) {
                    Ok(data) => {
                        json!({"content":[{"type":"text","text":data.to_string()}],"structuredContent":if data.is_object(){data}else{json!({"data":data})},"isError":false})
                    }
                    Err(e) => json!({"content":[{"type":"text","text":e}],"isError":true}),
                }
            }
            _ => return Some(error(id, -32601, "method not found")),
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}

pub fn serve(app: Application) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run(stdin.lock(), stdout.lock(), app.into_shared(), false)
}

/// Serve stdio on a desktop control thread. The GUI retains ownership of its
/// event loop; EOF simply ends this transport and does not close the window.
/// Write diagnostics to stderr so stdout remains exclusively JSON-RPC.
pub fn serve_shared(session: SharedSession) -> Result<(), String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_shared(stdin.lock(), stdout.lock(), session)
}

/// Reader/writer variant of the native desktop bridge, useful for embedding and
/// transport tests. No network listener or external process is required.
pub fn run_shared<R: BufRead, W: Write>(
    input: R,
    output: W,
    session: SharedSession,
) -> Result<(), String> {
    run(input, output, session, true)
}

fn run<R: BufRead, W: Write>(
    mut input: R,
    mut output: W,
    session: SharedSession,
    live_desktop: bool,
) -> Result<(), String> {
    let mut connection = Connection {
        session,
        live_desktop,
        initialized: false,
        ready: false,
    };
    while let Some(line) = read_message(&mut input).map_err(|e| e.to_string())? {
        let response = match line {
            Err(()) => Some(error(Value::Null, -32600, "message exceeds 1 MiB")),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(msg) => connection.handle(msg),
                Err(_) => Some(error(Value::Null, -32700, "parse error")),
            },
        };
        if let Some(response) = response {
            writeln!(output, "{response}").map_err(|e| e.to_string())?;
            output.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "openpatch-mcp-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn oversized_lines_recover_at_next_message() {
        let mut data = vec![b'x'; MAX_LINE + 2];
        data.extend_from_slice(b"\n{}\n");
        let mut r = io::Cursor::new(data);
        assert!(read_message(&mut r).unwrap().unwrap().is_err());
        assert_eq!(read_message(&mut r).unwrap().unwrap().unwrap(), b"{}\n");
    }

    #[test]
    fn desktop_transport_edits_the_gui_session_and_rejects_stale_revisions() {
        let workspace = workspace();
        let session = Application::new(&workspace).unwrap().into_shared();
        // A GUI edit is visible to the MCP client using this same session.
        session
            .lock()
            .unwrap()
            .replace_patch(openpatch_core::demo_patch(), Some(0))
            .unwrap();
        let requests = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":VERSIONS[0]}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inspect_patch"}}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"add_module","arguments":{"id":"desktop-test","kind":"constant","params":{"value":0.5},"expected_revision":1}}}),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"remove_module","arguments":{"id":"desktop-test","expected_revision":1}}}),
        ];
        let input = requests
            .iter()
            .map(|v| format!("{v}\n"))
            .collect::<String>();
        let mut output = Vec::new();
        run_shared(io::Cursor::new(input), &mut output, session.clone()).unwrap();
        let responses: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), 4);
        assert!(responses[0]["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("same live patch"));
        assert_eq!(responses[1]["result"]["structuredContent"]["revision"], 1);
        assert_eq!(responses[2]["result"]["structuredContent"]["revision"], 2);
        assert_eq!(responses[3]["result"]["isError"], true);
        let mut live = session.lock().unwrap();
        assert_eq!(live.revision(), 2);
        assert!(live.patch().nodes.iter().any(|n| n.id == "desktop-test"));
        // A GUI candidate based on its earlier view cannot silently lose MCP edits.
        assert!(live
            .replace_patch(openpatch_core::Patch::default(), Some(1))
            .unwrap_err()
            .contains("revision conflict"));
        let mut invalid = live.patch().clone();
        invalid.nodes.push(invalid.nodes[0].clone());
        assert!(live.replace_patch(invalid, Some(2)).is_err());
        assert_eq!(live.revision(), 2);
        drop(live);
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn shared_offline_render_uses_the_live_patch_without_changing_it() {
        let workspace = workspace();
        let session = Application::new(&workspace).unwrap().into_shared();
        let before = {
            let mut live = session.lock().unwrap();
            live.replace_patch(openpatch_core::demo_patch(), Some(0))
                .unwrap();
            live.snapshot()
        };
        let rendered = execute_shared(
            &session,
            "render_audio",
            json!({"filename":"snapshot.wav","seconds":0.01}),
        )
        .unwrap();
        assert_eq!(rendered["revision"], before["revision"]);
        assert_eq!(rendered["frames"], 480);
        assert!(rendered["peak"].as_f64().unwrap() > 0.0);
        assert_eq!(session.lock().unwrap().snapshot(), before);
        assert!(
            std::fs::metadata(workspace.join("snapshot.wav"))
                .unwrap()
                .len()
                > 44
        );
        std::fs::remove_dir_all(workspace).unwrap();
    }
}
