//! Exercise the real CLI bridge without launching a window or audio device.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use openpatch::{control::Application, local_mcp::LocalServer};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "openpatch-attach-integration-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        )))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Client {
    child: Child,
    responses: Receiver<Value>,
}
impl Client {
    fn new(id: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_openpatch"))
            .args(["attach", "--session", id])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self { child, responses }
    }
    fn send(&mut self, value: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }
    fn receive(&self) -> Value {
        self.responses
            .recv_timeout(Duration::from_secs(5))
            .expect("MCP response within deadline")
    }
    fn initialize(&mut self) {
        self.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}));
        assert_eq!(self.receive()["result"]["serverInfo"]["name"], "openpatch");
        self.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    }
    fn call(&mut self, name: &str, args: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}}));
        self.receive()
    }
    fn expect_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "bridge failed: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "bridge did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn two_cli_clients_share_edits_and_undo_but_not_handshakes() {
    let workspace = Workspace::new();
    let session = Application::new(&workspace.0).unwrap().into_shared();
    let server = LocalServer::start(session.clone(), &workspace.0).unwrap();
    let mut a = Client::new(server.session_id());
    let mut b = Client::new(server.session_id());
    a.initialize();
    // A's handshake cannot authorize B; both may safely reuse request IDs.
    assert_eq!(b.call("inspect_patch", json!({}))["error"]["code"], -32002);
    b.initialize();
    let added = a.call(
        "add_module",
        json!({"id":"shared-vco","kind":"constant","expected_revision":0}),
    );
    assert_eq!(added["result"]["isError"], false, "{added}");
    let seen = b.call("inspect_patch", json!({}));
    assert_eq!(seen["result"]["structuredContent"]["revision"], 1);
    assert!(seen.to_string().contains("shared-vco"));
    let stale = b.call(
        "add_module",
        json!({"id":"stale","kind":"constant","expected_revision":0}),
    );
    assert_eq!(stale["result"]["isError"], true);
    assert!(stale.to_string().contains("revision conflict"));
    let undo = b.call("undo", json!({"expected_revision":1}));
    assert_eq!(undo["result"]["isError"], false, "{undo}");
    assert!(session.lock().unwrap().patch().nodes.is_empty());

    // stdin EOF closes only this bridge; the other client and rack survive.
    drop(a.child.stdin.take());
    a.expect_exit();
    assert_eq!(
        b.call("inspect_patch", json!({}))["result"]["isError"],
        false
    );

    // Rack shutdown exits B even though its stdin remains open.
    assert!(b.child.stdin.is_some());
    drop(server);
    b.expect_exit();
}

#[test]
fn eighth_cli_client_can_attach_without_discovery_consuming_its_slot() {
    let workspace = Workspace::new();
    let session = Application::new(&workspace.0).unwrap().into_shared();
    let server = LocalServer::start(session, &workspace.0).unwrap();
    let mut clients = Vec::new();
    for _ in 0..8 {
        let mut client = Client::new(server.session_id());
        client.initialize();
        clients.push(client);
    }
    assert_eq!(server.client_count(), 8);
    assert_eq!(
        clients[7].call("inspect_patch", json!({}))["result"]["isError"],
        false
    );
    drop(server);
    for client in &mut clients {
        client.expect_exit();
    }
}

#[test]
fn workspace_ambiguity_and_missing_session_never_create_a_rack() {
    let workspace = Workspace::new();
    let a = LocalServer::start(
        Application::new(&workspace.0).unwrap().into_shared(),
        &workspace.0,
    )
    .unwrap();
    let b = LocalServer::start(
        Application::new(&workspace.0).unwrap().into_shared(),
        &workspace.0,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_openpatch"))
        .args(["attach", "--workspace"])
        .arg(&workspace.0)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        output.stdout.is_empty(),
        "diagnostics must not corrupt MCP stdout"
    );
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(
        message.contains(a.session_id()) && message.contains(b.session_id()),
        "{message}"
    );
    let id = a.session_id().to_owned();
    drop(a);
    let output = Command::new(env!("CARGO_BIN_EXE_openpatch"))
        .args(["attach", "--session", &id])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    drop(b);
}
