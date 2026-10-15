//! Bounded newline-delimited JSON-RPC over stdio and local Unix streams.
//! Listener ownership remains with the host; this module opens no listener.
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
    bounded_socket_jobs: bool,
}
// The socket host bounds client workers separately. These permits additionally
// bound CPU-heavy snapshots and preflight across socket connections, before copying a
// patch or acquiring the shared session lock. Stdio retains its original policy.
const MAX_SOCKET_JOBS: usize = 2;
static SOCKET_JOBS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct SocketJobGuard;
impl SocketJobGuard {
    fn acquire() -> Result<Self, String> {
        use std::sync::atomic::Ordering;
        let mut active = SOCKET_JOBS.load(Ordering::Relaxed);
        loop {
            if active >= MAX_SOCKET_JOBS {
                return Err(
                    "snapshot/preflight jobs busy; retry after an active job completes".into(),
                );
            }
            match SOCKET_JOBS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(Self),
                Err(current) => active = current,
            }
        }
    }
}
impl Drop for SocketJobGuard {
    fn drop(&mut self) {
        SOCKET_JOBS.fetch_sub(1, std::sync::atomic::Ordering::Release);
    }
}

fn expensive_job(name: &str) -> bool {
    matches!(
        name,
        "render_audio"
            | "render_arrangement"
            | "analyze_signal"
            | "capture_signal"
            | "benchmark"
            | "export_visualizer"
            | "start_construction"
    )
}

impl Connection {
    fn execute_tool(&self, name: &str, args: Value) -> Result<Value, String> {
        let _permit = if self.bounded_socket_jobs && expensive_job(name) {
            Some(SocketJobGuard::acquire()?)
        } else {
            None
        };
        execute_shared(&self.session, name, args)
    }

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
                    "OpenPatch is a native modular synthesizer. This connection controls the same live patch shown in the desktop window. Start with catalog and inspect_patch. For visible construction, start_construction preflights a bounded plan and commits paced steps; inspect_construction reports progress, and pause_construction/resume_construction/cancel_construction control future steps. Each committed step is one undo entry. External revision changes stop the plan; cancellation retains completed work. Plans do not start speaker playback. Edits update the window and its audio engine; speaker playback is controlled in the desktop window. Use expected_revision to avoid overwriting concurrent GUI edits. New patches use engine v2 with named outputs, up to 16 channels, and one sample per cable; inspect the patch version. analyze_signal measures any named output offline without changing the patch. capture_signal returns bounded peak-preserving waveform bins and calibrated channel statistics offline from reset; it is not a live scope. audio_stereo selects explicit L/R playback and WAV export; other outputs use the first channel. poly_sum mixes voices. render_audio, analyze_signal, and capture_signal accept sorted sample-frame midi_events and module_events to reproduce performances from reset without live playback or history edits. trigger_module queues live module actions, including sh_asr PUSH/CLEAR; its receipt confirms acceptance only. inspect_performance reports actual published module state, controls, lights, and callback counters. inspect_scope reads bounded coherent live rack Scope min/max captures for a selected voice; unavailable is distinct from zero, and there is no offline fallback. rotate_sequence commits one undoable column rotation. initialize_modules restores eligible defaults with audited per-kind initialization; randomize_modules requires a u32 seed and follows catalog eligibility/distributions. Both support atomic batches and preview; undo/redo restore saved controls and captured persistent state. capture_module_preset snapshots effective controls/current persistent state; save_module_preset writes exactly reviewed contents to a new .opreset file. Browse with list_module_presets/read_module_preset and apply reviewed inline contents with apply_module_preset; loading does not Initialize or replace the module. Filenames are confined to the configured workspace; existing files are never overwritten. No VCV plugin compatibility."
                } else {
                    "OpenPatch is an offline modular synthesizer. Start with catalog and inspect_patch. For visible construction, start_construction preflights a bounded plan and commits paced steps; inspect_construction reports progress, and pause_construction/resume_construction/cancel_construction control future steps. Each committed step is one undo entry. External revision changes stop the plan; cancellation retains completed work. Plans do not start speaker playback. New patches use engine v2 with named outputs and polyphonic cables. Build a patch, analyze_signal or capture_signal any output, then render_audio. capture_signal returns calibrated min/max waveform bins offline from reset, not live audio. audio_stereo selects stereo WAV export; other outputs use the first channel. Add poly_sum for a mono mix. Optional sorted sample-frame midi_events and module_events drive reproducible performances during render_audio, analyze_signal, and capture_signal without modifying the patch. trigger_module is unsupported without a live audio backend; inspect_performance reports that boundary. rotate_sequence commits one undoable seq3 column rotation. initialize_modules restores eligible defaults with audited per-kind initialization; randomize_modules requires a u32 seed and follows catalog eligibility/distributions. Both support atomic batches and preview; undo/redo restore saved controls and captured persistent state. capture_module_preset snapshots effective controls/current persistent state; save_module_preset writes exactly reviewed contents to a new .opreset file. Browse with list_module_presets/read_module_preset and apply reviewed inline contents with apply_module_preset; loading does not Initialize or replace the module. Filenames are confined to the configured workspace; existing files are never overwritten. This headless process owns an independent patch. No speaker playback or VCV plugin compatibility."
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
                match self.execute_tool(name, args) {
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
        bounded_socket_jobs: false,
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Clone, Copy)]
struct SocketLimits {
    handshake: std::time::Duration,
    frame: std::time::Duration,
    write: std::time::Duration,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn remaining(deadline: std::time::Instant) -> io::Result<std::time::Duration> {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "socket deadline exceeded"))
}

/// Socket frames require their newline. Once initialized, a completely idle
/// connection can wait indefinitely; the first received byte starts an absolute
/// incomplete-frame deadline that cannot be extended by trickling more bytes.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn read_socket_message(
    reader: &mut io::BufReader<std::os::unix::net::UnixStream>,
    handshake_deadline: Option<std::time::Instant>,
    frame_timeout: std::time::Duration,
) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut frame_deadline = None;
    loop {
        let deadline = match (handshake_deadline, frame_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        };
        reader
            .get_ref()
            .set_read_timeout(deadline.map(remaining).transpose()?)?;
        let buffer = match reader.fill_buf() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete socket frame",
                ))
            };
        }
        frame_deadline.get_or_insert_with(|| std::time::Instant::now() + frame_timeout);
        let count = buffer
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(buffer.len());
        if count > MAX_LINE - line.len() {
            // Unlike stdio, never drain an unbounded hostile frame. The socket
            // worker returns and shuts down both directions immediately.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "message exceeds 1 MiB",
            ));
        }
        let complete = buffer[count - 1] == b'\n';
        line.extend_from_slice(&buffer[..count]);
        reader.consume(count);
        if complete {
            // Check after the read as well: a queued newline arriving at the
            // deadline must not complete a timed-out handshake or partial frame.
            if let Some(deadline) = deadline {
                remaining(deadline)?;
            }
            return Ok(Some(line));
        }
    }
}

/// The write deadline applies to the entire serialized response, not each
/// partial write. A stalled peer cannot retain a worker by occasionally reading.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn write_socket_response(
    stream: &mut std::os::unix::net::UnixStream,
    response: &Value,
    deadline: std::time::Instant,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(response).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut written = 0;
    while written < bytes.len() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(&bytes[written..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "socket write returned zero",
                ))
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Serve one accepted local client against the desktop's shared session. Every
/// call owns fresh handshake state. Reads/writes occur only after control calls
/// release their session lock. Shutdown of a cloned socket wakes blocked I/O.
/// The host must separately bound accepted client threads (normally eight).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn run_socket(
    stream: std::os::unix::net::UnixStream,
    session: SharedSession,
) -> Result<(), String> {
    use std::time::Duration;
    run_socket_with_limits(
        stream,
        session,
        SocketLimits {
            handshake: Duration::from_secs(10),
            frame: Duration::from_secs(30),
            write: Duration::from_secs(10),
        },
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run_socket_with_limits(
    stream: std::os::unix::net::UnixStream,
    session: SharedSession,
    limits: SocketLimits,
) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| error.to_string())?;
    let handshake_deadline = std::time::Instant::now() + limits.handshake;
    let mut reader = io::BufReader::new(stream);
    let mut connection = Connection {
        session,
        live_desktop: true,
        initialized: false,
        ready: false,
        bounded_socket_jobs: true,
    };
    let result = (|| loop {
        let line = read_socket_message(
            &mut reader,
            (!connection.ready).then_some(handshake_deadline),
            limits.frame,
        )
        .map_err(|error| error.to_string())?;
        let Some(bytes) = line else {
            return Ok(());
        };
        let response = match serde_json::from_slice(&bytes) {
            Ok(message) => connection.handle(message),
            Err(_) => Some(error(Value::Null, -32700, "parse error")),
        };
        if let Some(response) = response {
            let write_deadline = std::time::Instant::now() + limits.write;
            let deadline = if connection.ready {
                write_deadline
            } else {
                write_deadline.min(handshake_deadline)
            };
            write_socket_response(reader.get_mut(), &response, deadline)
                .map_err(|error| error.to_string())?;
        }
    })();
    // Also close immediately on oversize/deadline errors. The server can use a
    // cloned descriptor's shutdown during teardown to wake these same reads.
    let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> std::path::PathBuf {
        // Clock resolution alone cannot distinguish parallel test threads.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "openpatch-mcp-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    mod socket_tests {
        use super::*;
        use std::net::Shutdown;
        use std::os::unix::net::UnixStream;
        use std::sync::mpsc;
        use std::thread;
        use std::time::{Duration, Instant};

        struct Client {
            reader: io::BufReader<UnixStream>,
            finished: mpsc::Receiver<Result<(), String>>,
            worker: Option<thread::JoinHandle<()>>,
        }
        impl Client {
            fn spawn(session: SharedSession, limits: SocketLimits) -> Self {
                let (server, client) = UnixStream::pair().unwrap();
                client
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                client
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let (sender, finished) = mpsc::channel();
                let worker = thread::spawn(move || {
                    let _ = sender.send(run_socket_with_limits(server, session, limits));
                });
                Self {
                    reader: io::BufReader::new(client),
                    finished,
                    worker: Some(worker),
                }
            }
            fn notify(&mut self, message: Value) {
                writeln!(self.reader.get_mut(), "{message}").unwrap();
            }
            fn request(&mut self, message: Value) -> Value {
                self.notify(message);
                let mut line = String::new();
                assert!(self.reader.read_line(&mut line).unwrap() > 0);
                serde_json::from_str(&line).unwrap()
            }
            fn initialize(&mut self) {
                let response = self.request(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                    "params":{"protocolVersion":VERSIONS[0]}}));
                assert!(response["result"]["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("same live patch"));
                self.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
            }
            fn result(&mut self) -> Result<(), String> {
                let result = self
                    .finished
                    .recv_timeout(Duration::from_secs(5))
                    .expect("socket worker did not wake");
                self.worker.take().unwrap().join().unwrap();
                result
            }
        }
        impl Drop for Client {
            fn drop(&mut self) {
                let _ = self.reader.get_ref().shutdown(Shutdown::Both);
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
            }
        }
        fn normal_limits() -> SocketLimits {
            SocketLimits {
                handshake: Duration::from_secs(10),
                frame: Duration::from_secs(2),
                write: Duration::from_secs(2),
            }
        }
        fn session() -> (SharedSession, std::path::PathBuf) {
            let directory = workspace();
            (
                Application::new(&directory).unwrap().into_shared(),
                directory,
            )
        }

        #[test]
        fn socket_handshake_deadline_is_absolute_and_requires_ready_notification() {
            let (session, directory) = session();
            let mut client = Client::spawn(
                session,
                SocketLimits {
                    handshake: Duration::from_millis(180),
                    ..normal_limits()
                },
            );
            // No ready notification: initialize and ping activity must not keep
            // this worker alive. A late write may find it correctly closed on
            // a busy runner; responses are not prerequisites for this check.
            let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"protocolVersion":VERSIONS[0]}});
            let _ = writeln!(client.reader.get_mut(), "{initialize}");
            for id in 2..5 {
                thread::sleep(Duration::from_millis(35));
                let ping = json!({"jsonrpc":"2.0","id":id,"method":"ping"});
                if writeln!(client.reader.get_mut(), "{ping}").is_err() {
                    break;
                }
            }
            assert!(client.result().is_err());
            std::fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn socket_expired_absolute_deadline_rejects_a_buffered_complete_frame() {
            let (server, mut peer) = UnixStream::pair().unwrap();
            peer.write_all(b"{}\n").unwrap();
            let mut reader = io::BufReader::new(server);
            assert_eq!(reader.fill_buf().unwrap(), b"{}\n");
            // An already available complete frame cannot restart an expired
            // handshake deadline. No sleep or cross-thread timing is needed.
            let deadline = Instant::now();
            let error = read_socket_message(&mut reader, Some(deadline), Duration::from_secs(30))
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert_eq!(reader.buffer(), b"{}\n");
        }

        #[test]
        fn socket_idle_initialized_client_survives_and_shutdown_wakes_reader() {
            let (session, directory) = session();
            let mut client = Client::spawn(
                session,
                SocketLimits {
                    handshake: Duration::from_secs(1),
                    frame: Duration::from_secs(1),
                    ..normal_limits()
                },
            );
            client.initialize();
            // Once ready, idle must outlive both deadlines. Setup has ample
            // scheduling margin; no writes are required during a short deadline.
            thread::sleep(Duration::from_millis(1200));
            assert!(matches!(
                client.finished.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            assert!(client
                .request(json!({"jsonrpc":"2.0","id":2,"method":"ping"}))
                .get("result")
                .is_some());
            client.reader.get_ref().shutdown(Shutdown::Both).unwrap();
            client.result().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn socket_partial_frame_deadline_does_not_reset_on_more_bytes() {
            let (server, mut peer) = UnixStream::pair().unwrap();
            peer.set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            // Start with available partial input: protocol setup and its multiple
            // writes must not compete with this deliberately short frame limit.
            peer.write_all(b"{").unwrap();
            let (sender, finished) = mpsc::channel();
            let worker = thread::spawn(move || {
                let mut reader = io::BufReader::new(server);
                let result = read_socket_message(&mut reader, None, Duration::from_millis(120));
                let _ = reader.get_ref().shutdown(Shutdown::Both);
                sender.send(result).unwrap();
            });
            for _ in 0..2 {
                thread::sleep(Duration::from_millis(35));
                // Scheduling can make this trickle arrive after the absolute
                // frame deadline. Peer closure is then the required outcome.
                if peer.write_all(b" ").is_err() {
                    break;
                }
            }
            let error = finished
                .recv_timeout(Duration::from_secs(5))
                .expect("partial-frame reader did not wake")
                .unwrap_err();
            assert!(matches!(
                error.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ));
            worker.join().unwrap();
        }

        #[test]
        fn socket_oversize_without_newline_closes_and_partial_eof_is_rejected() {
            let (shared, directory) = session();
            let mut client = Client::spawn(
                shared.clone(),
                SocketLimits {
                    handshake: Duration::from_secs(2),
                    frame: Duration::from_secs(2),
                    write: Duration::from_secs(2),
                },
            );
            let _ = client.reader.get_mut().write_all(&vec![b'x'; MAX_LINE + 1]);
            assert!(client.result().unwrap_err().contains("exceeds 1 MiB"));
            let mut partial = Client::spawn(shared, normal_limits());
            partial.reader.get_mut().write_all(b"{").unwrap();
            partial.reader.get_ref().shutdown(Shutdown::Write).unwrap();
            assert!(partial
                .result()
                .unwrap_err()
                .contains("incomplete socket frame"));
            std::fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn socket_client_handshakes_are_isolated_but_session_revisions_are_shared() {
            let (session, directory) = session();
            let limits = normal_limits();
            let mut first = Client::spawn(session.clone(), limits);
            let mut second = Client::spawn(session.clone(), limits);
            first.initialize();
            let unready = second.request(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"inspect_patch"}}));
            assert_eq!(unready["error"]["code"], -32002);
            let added = first.request(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"add_module","arguments":{"id":"socket-node","kind":"constant",
                    "params":{"value":0.5},"expected_revision":0}}}));
            assert_eq!(added["result"]["structuredContent"]["revision"], 1);
            second.initialize();
            let inspected = second.request(json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"inspect_patch"}}));
            assert_eq!(inspected["result"]["structuredContent"]["revision"], 1);
            let stale = second.request(json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
                "params":{"name":"remove_module","arguments":{"id":"socket-node","expected_revision":0}}}));
            assert_eq!(stale["result"]["isError"], true);
            assert_eq!(session.lock().unwrap().revision(), 1);
            assert!(
                first.request(json!({"jsonrpc":"2.0","id":5,"method":"initialize",
                "params":{"protocolVersion":VERSIONS[0]}}))["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("already initialized")
            );
            first.reader.get_ref().shutdown(Shutdown::Both).unwrap();
            second.reader.get_ref().shutdown(Shutdown::Both).unwrap();
            first.result().unwrap();
            second.result().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn socket_stalled_response_has_bounded_total_write_and_shutdown_wakes_writer() {
            let (mut server, client) = UnixStream::pair().unwrap();
            let start = Instant::now();
            let value = json!({"text":"x".repeat(4 * MAX_LINE)});
            assert!(
                write_socket_response(&mut server, &value, start + Duration::from_millis(100))
                    .is_err()
            );
            assert!(start.elapsed() < Duration::from_secs(1));
            drop(client);
            let (mut server, client) = UnixStream::pair().unwrap();
            let stopper = server.try_clone().unwrap();
            let (sender, finished) = mpsc::channel();
            let worker = thread::spawn(move || {
                sender
                    .send(write_socket_response(
                        &mut server,
                        &value,
                        Instant::now() + Duration::from_secs(10),
                    ))
                    .unwrap();
            });
            thread::sleep(Duration::from_millis(30));
            stopper.shutdown(Shutdown::Both).unwrap();
            assert!(finished
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err());
            worker.join().unwrap();
            drop(client);
        }

        #[test]
        fn socket_snapshot_jobs_are_global_bounded_and_busy_never_locks_session() {
            let (session, directory) = session();
            let first = SocketJobGuard::acquire().unwrap();
            let second = SocketJobGuard::acquire().unwrap();
            let mut connection = Connection {
                session: session.clone(),
                live_desktop: true,
                initialized: true,
                ready: true,
                bounded_socket_jobs: true,
            };
            // Holding this mutex proves rejection happens before snapshot/lock.
            let lock = session.lock().unwrap();
            for name in [
                "render_audio",
                "render_arrangement",
                "analyze_signal",
                "capture_signal",
                "benchmark",
                "export_visualizer",
                "start_construction",
            ] {
                let response = connection
                    .handle(json!({"jsonrpc":"2.0","id":1,
                    "method":"tools/call","params":{"name":name}}))
                    .unwrap();
                assert_eq!(response["result"]["isError"], true);
                assert!(response["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("busy"));
            }
            drop(lock);
            drop(first);
            // The permit is returned on ordinary tool errors as well as success.
            assert!(!connection
                .execute_tool("benchmark", json!({"seconds":0.0}))
                .unwrap_err()
                .contains("busy"));
            let replacement = SocketJobGuard::acquire().unwrap();
            assert!(SocketJobGuard::acquire().is_err());
            drop(replacement);
            drop(second);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}
