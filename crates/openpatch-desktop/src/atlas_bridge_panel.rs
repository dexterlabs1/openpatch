// Bridge is a view of native host authority, not an audio processor or a
// reconstruction of source pairing. Every duplicate observes the same session.
#[derive(Default)]
struct BridgeUi {
    status: Option<(Value, std::time::Instant)>,
    error: Option<String>,
    next_poll: Option<std::time::Instant>,
}

fn atlas_bridge_control(name: &str) -> Rect {
    let (left, top, right, bottom) = match name {
        "start" => (24.0, 286.0, 174.0, 316.0),
        "stop" => (186.0, 286.0, 336.0, 316.0),
        "restart" => (24.0, 324.0, 174.0, 354.0),
        "save" => (186.0, 324.0, 336.0, 354.0),
        _ => (24.0, 362.0, 336.0, 382.0),
    };
    Rect::from_min_max(pos2(left, top), pos2(right, bottom))
}

impl Rack {
    pub(crate) fn accept_bridge_status(&mut self, status: Value) {
        self.bridge_ui.status = Some((status, std::time::Instant::now()));
        self.bridge_ui.error = None;
    }

    pub(crate) fn bridge_status_error(&mut self, error: String) {
        self.bridge_ui.status = None;
        self.bridge_ui.error = Some(error);
    }

    pub(crate) fn bridge_poll_due(&mut self, patch: &Patch, revision: u64) -> bool {
        if self
            .bridge_ui
            .status
            .as_ref()
            .is_some_and(|(status, _)| status["revision"].as_u64() != Some(revision))
        {
            self.bridge_ui.status = None;
        }
        if !patch.nodes.iter().any(|node| node.kind == "rackmcp_bridge") {
            self.bridge_ui.status = None;
            self.bridge_ui.next_poll = None;
            return false;
        }
        let now = std::time::Instant::now();
        if self.bridge_ui.next_poll.is_some_and(|next| now < next) {
            return false;
        }
        self.bridge_ui.next_poll = Some(now + std::time::Duration::from_millis(100));
        true
    }

    fn atlas_bridge_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let p = |x, y| rect.min + vec2(x, y) * zoom;
        let text = |x, y, value: &str, size, color, align| {
            painter.text(
                p(x, y),
                align,
                value,
                crate::panel_skin::print_font(painter.ctx(), size * zoom),
                color,
            );
        };
        let bounded = |x, y, value: String, size, color, width| {
            let mut job = egui::text::LayoutJob::simple(
                value,
                crate::panel_skin::print_font(painter.ctx(), size * zoom),
                color,
                width * zoom,
            );
            job.wrap.max_rows = 1;
            let galley = painter.layout_job(job);
            painter.galley(p(x, y) - vec2(0.0, galley.size().y * 0.5), galley, color);
        };
        let status = self
            .bridge_ui
            .status
            .as_ref()
            .filter(|(status, received)| {
                interactive
                    && status["revision"].as_u64() == Some(revision)
                    && received.elapsed() < std::time::Duration::from_millis(500)
            })
            .map(|(status, _)| status);
        let transport = status.map(|status| &status["transport"]);
        let state = transport.and_then(|transport| transport["state"].as_str());
        let running = state == Some("running");
        let changing = state == Some("starting");
        let save = status.map(|status| &status["save"]);
        let save_state = save.and_then(|save| save["state"].as_str());
        let dirty = matches!(save_state, Some("modified" | "untitled"));
        let shared =
            status.is_some_and(|status| status["authority"]["kind"] == "same_user_shared_mutation");
        text(
            180.0,
            55.0,
            "NATIVE MCP · ONE SHARED SESSION",
            9.5,
            MUTED,
            Align2::CENTER_CENTER,
        );
        let unavailable =
            self.bridge_ui.error.as_deref().unwrap_or(
                "Native host status unavailable. Browser previews never start a bridge.",
            );
        for (y, label, detail, lit, color) in [
            (
                87.0,
                "LOCAL BRIDGE",
                match state {
                    Some("running") => "READY · LOCAL ATTACHMENT",
                    Some("starting") => "STARTING · WAIT FOR HOST",
                    Some("failed") => "FAILED · OPEN SESSION DETAILS",
                    Some("stopped") => "STOPPED · START TO ATTACH",
                    _ => "HOST UNAVAILABLE",
                },
                running,
                Color32::from_rgb(139, 198, 150),
            ),
            (
                132.0,
                "CONTROL AUTHORITY",
                if shared {
                    "SAME USER · SHARED MUTATION"
                } else {
                    "HOST UNAVAILABLE"
                },
                shared,
                Color32::from_rgb(143, 182, 202),
            ),
            (
                177.0,
                "PROJECT SAVE",
                match save_state {
                    Some("saved") => "SAVED · NATIVE CONTENT",
                    Some("modified") => "MODIFIED · SAVE CHANGES",
                    Some("untitled") => "UNTITLED · CHOOSE SAVE TARGET",
                    Some("unknown") => "UNKNOWN · OPEN SESSION DETAILS",
                    _ => "HOST UNAVAILABLE",
                },
                dirty,
                Color32::from_rgb(229, 183, 103),
            ),
        ] {
            let center = p(36.0, y);
            painter.circle_filled(center, 9.0 * zoom, Color32::from_rgb(91, 87, 72));
            self.lighting.circle(
                &painter,
                center,
                3.2 * zoom,
                color,
                if lit { 1.0 } else { 0.0 },
                0.09,
            );
            text(56.0, y - 7.0, label, 11.5, INK, Align2::LEFT_CENTER);
            text(56.0, y + 10.0, detail, 10.0, MUTED, Align2::LEFT_CENTER);
        }
        let draw = AtlasVoicePaint {
            painter: &painter,
            origin: rect.min,
            layout: AtlasVoiceLayout { width: 360.0 },
            zoom,
        };
        let start = crate::lighting::next(&painter);
        draw.screen(
            Rect::from_min_max(p(24.0, 207.0), p(336.0, 279.0)),
            &self.lighting,
        );
        let screen_ink = Color32::from_rgb(212, 211, 157);
        let clients = transport.map_or_else(
            || "LOCAL CLIENTS — · STDIO —".to_owned(),
            |transport| {
                format!(
                    "LOCAL CLIENTS {} · STDIO {}",
                    transport["admitted_clients"]
                        .as_u64()
                        .map_or_else(|| "—".to_owned(), |clients| clients.to_string()),
                    if transport["stdio_enabled"] == true {
                        "ENABLED"
                    } else {
                        "OFF"
                    }
                )
            },
        );
        bounded(34.0, 220.0, clients, 10.5, screen_ink, 292.0);
        let target = save.and_then(|save| save["target"].as_str()).map(|target| {
            std::path::Path::new(target)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(target)
        });
        bounded(
            34.0,
            241.0,
            target.map_or_else(
                || "SAVE TARGET · —".to_owned(),
                |target| format!("SAVE TARGET · {target}"),
            ),
            10.0,
            screen_ink,
            292.0,
        );
        let last = status
            .and_then(|status| status.get("last_operation"))
            .filter(|last| last.is_object());
        let last_text = last.map_or_else(
            || "LAST OPERATION · —".to_owned(),
            |last| {
                format!(
                    "{} · {} · {}",
                    last["name"].as_str().unwrap_or("Operation"),
                    last["origin"].as_str().unwrap_or("unknown"),
                    last["outcome"].as_str().unwrap_or("unknown")
                )
            },
        );
        bounded(34.0, 262.0, last_text, 9.5, screen_ink, 292.0);
        self.lighting.preserve(&painter, start);
        let mut action = None;
        for (name, label, enabled, hint) in [
            (
                "start",
                "START",
                status.is_some() && !running && !changing,
                "Start the native local MCP bridge for this shared session.",
            ),
            (
                "stop",
                "STOP",
                status.is_some() && running,
                "Disconnect local clients and stop the native attachment. Stdio is independent.",
            ),
            (
                "restart",
                "RESTART",
                status.is_some() && running,
                "Disconnect local clients and create a fresh native attachment.",
            ),
            (
                "save",
                "SAVE PROJECT",
                status.is_some() && !changing,
                "Save through the native project command. Untitled projects open Save As.",
            ),
            (
                "show",
                "SESSION DETAILS",
                status.is_some(),
                "Open native connection details, save status and shared authority information.",
            ),
        ] {
            let local = atlas_bridge_control(name);
            let bounds =
                Rect::from_min_max(p(local.min.x, local.min.y), p(local.max.x, local.max.y));
            let response = ui.interact(
                bounds,
                ui.id().with((&node.id, "native-bridge", name)),
                if interactive && enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            painter.rect_filled(
                bounds,
                3.0 * zoom,
                if response.hovered() && enabled {
                    Color32::from_rgb(237, 227, 198)
                } else {
                    Color32::from_rgb(222, 211, 180)
                },
            );
            painter.rect_stroke(
                bounds,
                3.0 * zoom,
                Stroke::new(zoom, MUTED),
                egui::StrokeKind::Inside,
            );
            text(
                local.center().x,
                local.center().y,
                label,
                if name == "show" { 10.5 } else { 12.0 },
                if enabled { INK } else { MUTED },
                Align2::CENTER_CENTER,
            );
            if response.clicked() {
                let command = match name {
                    "start" => "_native_bridge_start",
                    "stop" => "_native_bridge_stop",
                    "restart" => "_native_bridge_restart",
                    "save" => "_native_bridge_save",
                    _ => "_native_bridge_show",
                };
                action = Some(RackAction::Command(
                    command,
                    json!({"id":node.id,"expected_revision":revision}),
                ));
            }
            response.on_hover_text(if status.is_some() { hint } else { unavailable });
        }
        action
    }
}
