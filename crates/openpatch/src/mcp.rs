//! Bounded newline-delimited JSON-RPC over stdio. No network listener or model API.
use crate::control::{tools, Application};
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

struct Session {
    app: Application,
    initialized: bool,
    ready: bool,
}
impl Session {
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
                json!({"protocolVersion":version,"capabilities":{"tools":{}},"serverInfo":{"name":"openpatch","version":env!("CARGO_PKG_VERSION")},"instructions":"OpenPatch is an offline modular synthesizer. Start with catalog and inspect_patch. Build a patch, select its output, then render_audio. Filenames are confined to the configured workspace; existing files are never overwritten. Each connection has an independent patch. No speaker playback or VCV plugin compatibility yet."})
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
                match self.app.execute(name, args) {
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
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let mut session = Session {
        app,
        initialized: false,
        ready: false,
    };
    while let Some(line) = read_message(&mut input).map_err(|e| e.to_string())? {
        let response = match line {
            Err(()) => Some(error(Value::Null, -32600, "message exceeds 1 MiB")),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(msg) => session.handle(msg),
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
    #[test]
    fn oversized_lines_recover_at_next_message() {
        let mut data = vec![b'x'; MAX_LINE + 2];
        data.extend_from_slice(b"\n{}\n");
        let mut r = io::Cursor::new(data);
        assert!(read_message(&mut r).unwrap().unwrap().is_err());
        assert_eq!(read_message(&mut r).unwrap().unwrap().unwrap(), b"{}\n");
    }
}
