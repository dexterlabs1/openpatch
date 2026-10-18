use super::*;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use openpatch::control::execute_shared;
use openpatch::control::Session;
use serde_json::{json, Value};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::time::{Duration, Instant};

fn session() -> (tempfile::TempDir, SharedSession) {
    let workspace = tempfile::tempdir().unwrap();
    let session = Session::new(workspace.path()).unwrap().into_shared();
    (workspace, session)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn eventually(check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < deadline, "condition did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn real_bridge_handshake_stop_and_restart_preserve_patch_and_history() {
    use std::io::{BufRead, BufReader, Read, Write};
    let (workspace, shared) = session();
    execute_shared(
        &shared,
        "add_module",
        json!({"id":"source","kind":"constant"}),
    )
    .unwrap();
    let before = shared.lock().unwrap().snapshot();
    let mut panel = Panel::new(true);
    panel.start(&shared, workspace.path());
    let first = panel.session_id().unwrap().to_owned();
    assert_eq!(panel.summary(), "MCP ready · 0 clients");
    panel.start(&shared, workspace.path());
    assert_eq!(
        panel.session_id(),
        Some(first.as_str()),
        "start is idempotent"
    );
    let mut stream = openpatch::local_mcp::connect(Some(&first), None).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writeln!(stream, "{}", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"panel test","version":"1"}}})).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 1);
    assert!(response["result"]["serverInfo"].is_object());
    eventually(|| panel.clients() == 1);
    assert_eq!(panel.summary(), "MCP connected · 1 client");
    panel.stop(&shared);
    assert!(panel.server.is_none());
    assert!(panel.error.is_none());
    assert!(panel.stdio, "independent stdio launch state is retained");
    match stream.read(&mut [0]) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
            ) => {}
        result => panic!("attached stream did not disconnect: {result:?}"),
    }
    assert!(openpatch::local_mcp::connect(Some(&first), None).is_err());
    assert_eq!(before, shared.lock().unwrap().snapshot());
    panel.start(&shared, workspace.path());
    assert_ne!(panel.session_id().unwrap(), first);
    panel.shutdown();
    assert!(panel.session_id().is_none());
    assert_eq!(before, shared.lock().unwrap().snapshot());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn stop_pauses_running_construction_without_another_edit() {
    let (workspace, shared) = session();
    let mut panel = Panel::new(false);
    panel.start(&shared, workspace.path());
    execute_shared(&shared, "start_construction", json!({
        "expected_revision":0,"interval_ms":2000,
        "steps":[
            {"label":"Add source","edits":[{"tool":"add_module","arguments":{"id":"source","kind":"constant","params":{"value":0}}}]},
            {"label":"Set source","edits":[{"tool":"set_parameter","arguments":{"id":"source","parameter":"value","value":1}}]}
        ]
    })).unwrap();
    eventually(|| shared.lock().unwrap().construction_snapshot()["completed_steps"] == 1);
    let before = shared.lock().unwrap().snapshot();
    panel.stop(&shared);
    let live = shared.lock().unwrap();
    assert_eq!(live.construction_snapshot()["status"], "paused");
    assert_eq!(before, live.snapshot());
    assert!(panel.server.is_none());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn errors_are_recoverable_and_busy_stop_retains_the_bridge() {
    let (workspace, shared) = session();
    let mut panel = Panel::new(false);
    panel.start(&shared, &workspace.path().join("missing"));
    assert!(panel.error.is_some());
    assert!(panel.session_id().is_none());
    panel.start(&shared, workspace.path());
    assert!(panel.error.is_none());
    let id = panel.session_id().unwrap().to_owned();
    let guard = shared.lock().unwrap();
    panel.stop(&shared);
    assert_eq!(panel.session_id(), Some(id.as_str()));
    assert!(panel.error.as_ref().unwrap().contains("busy"));
    drop(guard);
    panel.stop(&shared);
    assert!(panel.error.is_none());
    assert!(panel.server.is_none());
}

#[test]
fn copied_configurations_preserve_paths_and_arguments_exactly() {
    // Every troublesome shell/config character remains data in all three forms.
    let path = Path::new("/Applications/Open Patch's \\\"Studio\\\"/$HOME`whoami`\n雪/openpatch");
    let id = "0123456789abcdef0123456789abcdef";
    let instructions = Instructions::new(path, id).unwrap();
    let json: Value = serde_json::from_str(&instructions.claude).unwrap();
    assert_eq!(
        json["mcpServers"]["openpatch"]["command"],
        path.to_str().unwrap()
    );
    assert_eq!(
        json["mcpServers"]["openpatch"]["args"],
        json!(["attach", "--session", id])
    );
    let toml_command = instructions
        .codex
        .lines()
        .find_map(|line| line.strip_prefix("command = "))
        .unwrap();
    // The basic-string subset emitted here is also valid JSON, allowing a
    // parser to verify escapes without adding a TOML dependency to the app.
    assert_eq!(
        serde_json::from_str::<String>(toml_command).unwrap(),
        path.to_str().unwrap()
    );
    let args = instructions
        .codex
        .lines()
        .find_map(|line| line.strip_prefix("args = "))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(args).unwrap(),
        json!(["attach", "--session", id])
    );
    assert_eq!(
        decode_posix_words(&instructions.command),
        vec![path.to_str().unwrap(), "attach", "--session", id]
    );
    for character in (0..=31).chain(std::iter::once(127)) {
        let value = format!("a{}b", char::from_u32(character).unwrap());
        assert_eq!(
            serde_json::from_str::<String>(&toml_quote(&value)).unwrap(),
            value
        );
    }
}

// Parse emitted POSIX quoting without executing any command or external app.
fn decode_posix_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            word.push(character);
            escaped = false;
        } else if character == '\\' && !quoted {
            escaped = true;
        } else if character == '\'' {
            quoted = !quoted;
        } else if character == ' ' && !quoted {
            words.push(std::mem::take(&mut word));
        } else {
            word.push(character);
        }
    }
    assert!(!quoted && !escaped);
    words.push(word);
    words
}

struct Host {
    context: egui::Context,
    panel: Panel,
    shared: SharedSession,
    workspace: tempfile::TempDir,
    time: f64,
    copies: Option<Instructions>,
}

impl Host {
    fn new() -> Self {
        let (workspace, shared) = session();
        Self {
            context: egui::Context::default(),
            panel: Panel::new(false),
            shared,
            workspace,
            time: 0.0,
            copies: None,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.05;
        self.context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 680.0),
                )),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::TopBottomPanel::top("test_toolbar").show(ctx, |ui| {
                    self.panel.toolbar(ui, &self.shared, self.workspace.path());
                });
                egui::TopBottomPanel::bottom("test_footer").show(ctx, |ui| self.panel.status(ui));
                if let Some(instructions) = &self.copies {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        copy_section(
                            ui,
                            "Attach command",
                            "Copy attach command",
                            &instructions.command,
                        );
                        copy_section(
                            ui,
                            "Claude Desktop JSON",
                            "Copy Claude JSON",
                            &instructions.claude,
                        );
                        copy_section(ui, "Codex TOML", "Copy Codex TOML", &instructions.codex);
                    });
                }
                self.panel.show(ctx, &self.shared, self.workspace.path());
            },
        )
    }

    fn click(&mut self, label: &str) -> egui::FullOutput {
        self.frame(vec![]);
        let settled = self.frame(vec![]);
        let point = text_rect(&settled, label)
            .unwrap_or_else(|| panic!("missing control {label}"))
            .center();
        self.frame(vec![egui::Event::PointerMoved(point)]);
        self.frame(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        self.frame(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }])
    }
}

fn text_rect(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
    fn find(shape: &egui::Shape, label: &str, clip: egui::Rect) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                clip.contains(rect.center()).then_some(rect)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, label, clip)),
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|shape| find(&shape.shape, label, shape.clip_rect))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn toolbar_starts_and_opens_real_bridge_and_footer_reopens_details() {
    let mut host = Host::new();
    assert!(!host.panel.is_open());
    host.click("Start MCP bridge");
    assert!(host.panel.is_open());
    let id = host.panel.session_id().unwrap().to_owned();
    assert_eq!(host.panel.clients(), 0);
    let output = host.frame(vec![]);
    assert!(text_rect(&output, "MCP session").is_some());
    assert!(text_rect(&output, "Stop bridge").is_some());
    host.panel.open = false;
    host.click("MCP ready · 0 clients");
    assert!(host.panel.is_open());
    assert_eq!(host.panel.session_id(), Some(id.as_str()));
    host.panel.shutdown();
}

#[test]
fn details_can_open_without_starting_and_disabled_copy_does_nothing() {
    let mut host = Host::new();
    host.panel.open();
    let before = host.shared.lock().unwrap().snapshot();
    let output = host.click("Copy attach command");
    assert!(output.platform_output.commands.is_empty());
    assert!(host.panel.is_open());
    assert!(host.panel.session_id().is_none());
    assert_eq!(before, host.shared.lock().unwrap().snapshot());
}

#[test]
fn copy_buttons_emit_exact_clipboard_data_and_support_keyboard_focus() {
    let mut host = Host::new();
    host.copies = Some(
        Instructions::new(
            Path::new("/an app's folder/openpatch"),
            "0123456789abcdef0123456789abcdef",
        )
        .unwrap(),
    );
    let before = host.shared.lock().unwrap().snapshot();
    for (label, expected) in [
        (
            "Copy attach command",
            host.copies.as_ref().unwrap().command.clone(),
        ),
        (
            "Copy Claude JSON",
            host.copies.as_ref().unwrap().claude.clone(),
        ),
        (
            "Copy Codex TOML",
            host.copies.as_ref().unwrap().codex.clone(),
        ),
    ] {
        let output = host.click(label);
        assert!(output.platform_output.commands.iter().any(
            |command| matches!(command,egui::OutputCommand::CopyText(text) if text == &expected)
        ));
    }
    host.frame(vec![egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(
        host.context.memory(|memory| memory.focused()).is_some(),
        "controls must be keyboard focusable"
    );
    assert_eq!(before, host.shared.lock().unwrap().snapshot());
}

#[test]
fn mcp_details_dropup_anchors_to_footer_and_dismisses_without_starting_bridge() {
    let mut host = Host::new();
    let before = host.shared.lock().unwrap().snapshot();
    host.click("MCP off");
    let output = host.frame(vec![]);
    assert!(host.panel.is_open());
    assert!(text_rect(&output, "MCP session").is_some());
    let popup = host
        .context
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::new("mcp-session-dropdown-rect")))
        .unwrap();
    let anchor = host.panel.anchor.unwrap().0;
    assert!(popup.bottom() <= anchor.top());
    assert!(
        popup.left() >= 0.0 && popup.left() <= anchor.left() + 5.0,
        "screen edge clamps the footer-anchored panel"
    );
    host.click("MCP off");
    assert!(!host.panel.is_open());
    host.click("MCP off");
    host.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(!host.panel.is_open());
    host.click("MCP off");
    let position = egui::pos2(900.0, 150.0);
    for pressed in [true, false] {
        host.frame(vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
    }
    assert!(!host.panel.is_open());
    assert!(host.panel.session_id().is_none());
    assert_eq!(before, host.shared.lock().unwrap().snapshot());
}
