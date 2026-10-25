//! Controls for attaching external MCP clients to this desktop's live session.
use eframe::egui::{self, RichText};
use openpatch::control::SharedSession;
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use openpatch::local_mcp::LocalServer;

#[cfg(test)]
#[path = "mcp_session_ui_tests.rs"]
mod tests;

pub(crate) struct Panel {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    server: Option<LocalServer>,
    stdio: bool,
    open: bool,
    anchor: Option<(egui::Rect, egui::LayerId, bool)>,
    error: Option<String>,
}

impl Panel {
    pub(crate) fn new(stdio: bool) -> Self {
        Self {
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            server: None,
            stdio,
            open: false,
            anchor: None,
            error: None,
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn set_server(&mut self, result: Result<LocalServer, String>) {
        match result {
            Ok(server) => {
                if self
                    .server
                    .as_ref()
                    .is_none_or(|server| !server.is_running())
                {
                    self.server = Some(server);
                }
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn shutdown(&mut self) {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.server = None;
        }
    }

    fn session_id(&self) -> Option<&str> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.server
                .as_ref()
                .filter(|server| server.is_running())
                .map(LocalServer::session_id)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }

    fn clients(&self) -> usize {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.server
                .as_ref()
                .filter(|server| server.is_running())
                .map_or(0, LocalServer::client_count)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0
        }
    }

    fn listener_failed(&self) -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.server
                .as_ref()
                .is_some_and(|server| !server.is_running())
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            false
        }
    }
    fn error_message(&self) -> Option<String> {
        let error = self.error.clone();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            error.or_else(|| self.server.as_ref().and_then(LocalServer::failure_reason))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            error
        }
    }
    fn summary(&self) -> String {
        if self.session_id().is_some() {
            let clients = self.clients();
            if clients == 0 {
                "MCP ready · 0 clients".into()
            } else {
                format!(
                    "MCP connected · {clients} client{}",
                    if clients == 1 { "" } else { "s" }
                )
            }
        } else if self.error.is_some() || self.listener_failed() {
            "MCP · needs attention".into()
        } else if self.stdio {
            "MCP · stdio enabled".into()
        } else {
            "MCP off".into()
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn start(&mut self, session: &SharedSession, workspace: &Path) {
        if self.server.as_ref().is_some_and(LocalServer::is_running) {
            self.error = None;
            return;
        }
        if self.server.is_some() {
            self.stop(session);
            if self.error.is_some() {
                return;
            }
        }
        if let Ok(live) = session.try_lock() {
            live.bridge_observation().set_stdio_enabled(self.stdio);
        }
        self.set_server(LocalServer::start(session.clone(), workspace));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn stop(&mut self, session: &SharedSession) {
        if self.server.is_none() {
            return;
        }
        // Hold the same serialization boundary as construction commits. A busy
        // or poisoned session must leave the bridge intact for a safe retry.
        let result = (|| {
            let mut live = session.try_lock().map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => {
                    "The session is busy. Retry Stop bridge when the current operation finishes."
                        .to_owned()
                }
                std::sync::TryLockError::Poisoned(_) => {
                    "The session lock is unavailable. The bridge was kept running.".to_owned()
                }
            })?;
            let construction = live.construction_snapshot();
            if construction["status"] == "running" {
                live.execute(
                    "pause_construction",
                    serde_json::json!({"plan_id":construction["plan_id"]}),
                )
                .map_err(|error| {
                    format!("Could not pause construction; bridge kept running: {error}")
                })?;
            }
            // Disconnect local streams while commits are excluded. Stdio is
            // owned by main and deliberately independent of this local bridge.
            self.server = None;
            Ok::<(), String>(())
        })();
        self.error = result.err();
    }

    pub(crate) fn bridge_action(
        &mut self,
        action: &str,
        session: &SharedSession,
        workspace: &Path,
    ) -> Result<(), String> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            match action {
                "start" => self.start(session, workspace),
                "stop" => {
                    self.error = None;
                    self.stop(session);
                }
                "restart" => {
                    self.error = None;
                    self.stop(session);
                    if self.error.is_none() {
                        self.start(session, workspace);
                    }
                }
                _ => return Err("unknown native bridge action".into()),
            }
            let result = self.error.clone().map_or(Ok(()), Err);
            if let Ok(mut live) = session.try_lock() {
                live.record_bridge_action(action, result.is_ok());
            }
            result
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (action, session, workspace);
            Err("local MCP attachment unavailable on this platform".into())
        }
    }

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui, session: &SharedSession, workspace: &Path) {
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let _ = (session, workspace);
        let label = if self.session_id().is_some() && self.clients() > 0 {
            "AI connected"
        } else {
            "Connect AI"
        };
        let response = ui
            .button(label)
            .on_hover_text("Let Claude or Codex edit this live rack over MCP");
        if self.open && self.anchor.is_some_and(|(_, _, above)| !above) {
            self.anchor = Some((response.rect, response.layer_id, false));
        }
        if response.clicked() {
            if self.session_id().is_none() {
                #[cfg(any(target_os = "macos", target_os = "linux"))]
                let _ = self.bridge_action("start", session, workspace);
            }
            self.anchor = Some((response.rect, response.layer_id, false));
            self.open = !self.open;
        }
    }

    /// Nothing to report: no connected client and no error.
    pub(crate) fn is_idle(&self) -> bool {
        !self.open
            && self.error_message().is_none()
            && (self.session_id().is_none() || self.clients() == 0)
    }

    pub(crate) fn status(&mut self, ui: &mut egui::Ui) {
        let summary = self.summary();
        let response = ui
            .add(egui::Button::new(RichText::new(&summary).small()).frame(false))
            .on_hover_text(format!("{summary}\nOpen MCP session details"));
        if self.open && self.anchor.is_some_and(|(_, _, above)| above) {
            self.anchor = Some((response.rect, response.layer_id, true));
        }
        if response.clicked() {
            self.anchor = Some((response.rect, response.layer_id, true));
            self.open = !self.open;
        }
    }

    pub(crate) fn show(&mut self, ctx: &egui::Context, session: &SharedSession, workspace: &Path) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let (anchor, layer, above) = self.anchor.unwrap_or((
            egui::Rect::from_min_size(
                ctx.content_rect().right_top() + egui::vec2(-110.0, 10.0),
                egui::vec2(90.0, 30.0),
            ),
            egui::LayerId::background(),
            false,
        ));
        let width = 500.0_f32.min(ctx.content_rect().width() - 28.0).max(240.0);
        let height = if above {
            anchor.top() - ctx.content_rect().top()
        } else {
            ctx.content_rect().bottom() - anchor.bottom()
        };
        let response = egui::Popup::new(egui::Id::new("mcp-session-dropdown"), ctx.clone(), anchor, layer)
            .kind(egui::PopupKind::Menu)
            .open_bool(&mut open)
            .align(if above { egui::emath::RectAlign::TOP_END } else { egui::emath::RectAlign::BOTTOM_END })
            .align_alternatives(&[])
            .gap(6.0)
            .width(width)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(width);
                egui::ScrollArea::vertical().id_salt("mcp-session-scroll").max_height((height - 30.0).clamp(80.0, 570.0)).show(ui, |ui| {
                    ui.heading("MCP session");
                    ui.label(RichText::new(self.summary()).strong());
                    ui.label("Use Claude or Codex to inspect and build the patch in this window. Connect in your client, then ask it to inspect the rack.");
                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    {
                        ui.horizontal(|ui| {
                            if self.session_id().is_some() {
                                if ui.button("Stop bridge").clicked() {
                                    let _ = self.bridge_action("stop", session, workspace);
                                }
                            } else {
                                let label = if self.error.is_some() { "Retry" } else { "Start bridge" };
                                if ui.button(label).clicked() {
                                    let _ = self.bridge_action("start", session, workspace);
                                }
                            }
                        });
                        ui.small("Stopping pauses running construction and disconnects local clients. Completed edits and playback stay as they are.");
                    }
                    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                    {
                        let _ = (session, workspace);
                        ui.label("Local attachment is available on macOS and Linux. On this platform, configure your client to launch openpatch-desktop --mcp over stdio.");
                    }
                    if self.stdio {
                        ui.small("Stdio was enabled at launch. It is independent of this bridge; its connection status is not tracked here. Stop bridge leaves stdio available.");
                    }
                    if let Some(error) = self.error_message() {
                        ui.label(RichText::new(error).color(ui.visuals().error_fg_color));
                    }
                    ui.separator();
                    if let Some(id) = self.session_id() {
                        ui.label("This window's session");
                        ui.monospace(id);
                        ui.small("The session ID changes when the bridge restarts. Copy an updated configuration after restarting.");
                    }
                    let instructions = self.session_id().map(|id| {
                        attach_executable().and_then(|executable| Instructions::new(&executable, id))
                    });
                    match instructions {
                        Some(Ok(instructions)) => {
                            copy_section(ui, "Attach command", "Copy attach command", &instructions.command);
                            copy_section(ui, "Claude Desktop JSON", "Copy Claude JSON", &instructions.claude);
                            copy_section(ui, "Codex TOML", "Copy Codex TOML", &instructions.codex);
                            ui.small("Paste the configuration in your client's MCP settings and reconnect. These snippets attach to this window while its bridge is running.");
                        }
                        unavailable => {
                            if let Some(Err(error)) = unavailable {
                                ui.label(RichText::new(error).color(ui.visuals().warn_fg_color));
                            } else {
                                #[cfg(any(target_os = "macos", target_os = "linux"))]
                                ui.label("Start the bridge to get this window's attachment details.");
                                #[cfg(not(any(target_os = "macos", target_os = "linux")))]
                                ui.label("Copied attachment details require a local bridge on macOS or Linux.");
                            }
                            for label in ["Copy attach command", "Copy Claude JSON", "Copy Codex TOML"] {
                                ui.add_enabled(false, egui::Button::new(label));
                            }
                        }
                    }
                    ui.separator();
                    ui.small("Clients share this rack and its Undo history. Review changes in the rack; playback is controlled here.");
                });
            });
        if let Some(response) = response {
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("mcp-session-dropdown-rect"),
                    response.response.rect,
                )
            });
        }
        self.open = open;
        if self.session_id().is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

fn copy_section(ui: &mut egui::Ui, title: &str, button: &str, contents: &str) {
    ui.horizontal(|ui| {
        if ui.button(button).clicked() {
            ui.ctx().copy_text(contents.to_owned());
        }
        egui::CollapsingHeader::new(title)
            .id_salt(title)
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .max_height(140.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(contents).monospace())
                                .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            });
    });
}

fn attach_executable() -> Result<std::path::PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| format!("Locate OpenPatch: {e}"))?;
    let executable = executable.with_file_name(if cfg!(windows) {
        "openpatch.exe"
    } else {
        "openpatch"
    });
    if !executable.is_file() {
        return Err(format!("Install the openpatch CLI beside this desktop app to copy attachment details. Expected: {}", executable.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if executable
            .metadata()
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(format!(
                "The OpenPatch CLI is not executable: {}",
                executable.display()
            ));
        }
    }
    Ok(executable)
}

struct Instructions {
    command: String,
    claude: String,
    codex: String,
}

impl Instructions {
    fn new(executable: &Path, session_id: &str) -> Result<Self, String> {
        let executable = executable
            .to_str()
            .ok_or("The OpenPatch CLI path must be valid UTF-8 to copy client configuration.")?;
        let arguments = ["attach", "--session", session_id];
        let claude = serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": { "openpatch": {"command":executable,"args":arguments} }
        }))
        .map_err(|e| e.to_string())?;
        let command = std::iter::once(executable)
            .chain(arguments)
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ");
        let codex = format!(
            "[mcp_servers.openpatch]\ncommand = {}\nargs = [{}, {}, {}]\n",
            toml_quote(executable),
            toml_quote(arguments[0]),
            toml_quote(arguments[1]),
            toml_quote(arguments[2])
        );
        Ok(Self {
            command,
            claude,
            codex,
        })
    }
}

// POSIX shell quoting is used only for copied text, never to launch a process.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn toml_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\u{8}' => quoted.push_str("\\b"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\u{c}' => quoted.push_str("\\f"),
            '\r' => quoted.push_str("\\r"),
            c if c <= '\u{1f}' || c == '\u{7f}' => quoted.push_str(&format!("\\u{:04X}", c as u32)),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}
