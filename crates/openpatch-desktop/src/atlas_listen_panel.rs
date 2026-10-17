// Native capture controls are host operations. They never author DSP parameters.
struct ListenUi {
    seconds: f64,
    write_wav: bool,
    duration_menu: Option<String>,
    inspected: Option<String>,
    statuses: BTreeMap<String, Value>,
    capture: Option<Value>,
    error: Option<String>,
    next_poll: Option<std::time::Instant>,
}

impl Default for ListenUi {
    fn default() -> Self {
        Self {
            seconds: 1.0,
            write_wav: false,
            duration_menu: None,
            inspected: None,
            statuses: BTreeMap::new(),
            capture: None,
            error: None,
            next_poll: None,
        }
    }
}

fn listen_capture_active(capture: &Value) -> bool {
    matches!(capture["state"].as_str(), Some("capturing" | "analyzing"))
}

fn listen_analysis_rows(analysis: &Value) -> Vec<String> {
    let mut rows = Vec::new();
    for (index, side) in ["L", "R"].iter().enumerate() {
        let channel = &analysis["channels"][index];
        rows.push(if channel["present"] == true {
            format!(
                "{side} · peak {:.3} V · RMS {:.3} V · DC {:.3} V",
                channel["peakVolts"].as_f64().unwrap_or(0.0),
                channel["rmsVolts"].as_f64().unwrap_or(0.0),
                channel["dcVolts"].as_f64().unwrap_or(0.0)
            )
        } else {
            format!("{side} · absent")
        });
    }
    for (label, valid, field, unit) in [
        ("Loudness", "loudnessValid", "integratedLufs", "LUFS"),
        ("Tempo", "tempoValid", "tempoBpm", "BPM"),
        ("Pitch", "pitchValid", "pitchHz", "Hz"),
    ] {
        rows.push(if analysis[valid] == true {
            analysis[field].as_f64().map_or_else(
                || format!("{label} · unavailable"),
                |value| format!("{label} · {value:.2} {unit}"),
            )
        } else {
            format!("{label} · unavailable")
        });
    }
    rows
}

fn atlas_listen_port(input: bool, index: usize) -> Pos2 {
    pos2(
        70.0 + index as f32 * 220.0,
        if input { 126.0 } else { 215.0 },
    )
}

fn atlas_listen_control(name: &str) -> Rect {
    match name {
        "duration" => Rect::from_min_max(pos2(24.0, 275.0), pos2(137.0, 307.0)),
        "wav" => Rect::from_min_max(pos2(149.0, 275.0), pos2(336.0, 307.0)),
        "capture" => Rect::from_min_max(pos2(24.0, 321.0), pos2(172.0, 355.0)),
        _ => Rect::from_min_max(pos2(188.0, 321.0), pos2(336.0, 355.0)),
    }
}

impl Rack {
    pub(crate) fn listen_capture_pending(&self) -> bool {
        self.listen_ui.capture.as_ref().is_some_and(listen_capture_active)
    }
    pub(crate) fn listen_poll_commands(
        &mut self,
        patch: &Patch,
        revision: u64,
    ) -> Vec<(&'static str, Value)> {
        let now = std::time::Instant::now();
        if self.listen_ui.next_poll.is_some_and(|next| now < next) {
            return Vec::new();
        }
        self.listen_ui.next_poll = Some(now + std::time::Duration::from_millis(100));
        self.listen_ui.statuses.retain(|id, _| {
            patch
                .nodes
                .iter()
                .any(|n| n.id == *id && n.kind == "rackmcp_listen")
        });
        let mut commands: Vec<_> = patch
            .nodes
            .iter()
            .filter(|n| n.kind == "rackmcp_listen")
            .map(|n| {
                (
                    "inspect_listen",
                    json!({"id":n.id,"expected_revision":revision}),
                )
            })
            .collect();
        if let Some(capture) = self
            .listen_ui
            .capture
            .as_ref()
            .filter(|capture| listen_capture_active(capture))
        {
            commands.push((
                "inspect_listen_capture",
                json!({"job":capture["job"],"expected_revision":revision}),
            ));
        }
        commands
    }

    pub(crate) fn accept_listen_receipt(&mut self, command: &str, receipt: Value) {
        if command == "inspect_listen" {
            if let Some(capture) = receipt.get("capture").filter(|capture| capture.is_object()) {
                self.accept_listen_receipt("inspect_listen_capture", json!({"capture":capture}));
            }
            if let Some(id) = receipt["id"].as_str() {
                self.listen_ui.statuses.insert(id.to_owned(), receipt);
            }
        } else if let Some(capture) = receipt.get("capture") {
            let mut capture = capture.clone();
            if capture.get("id").is_none() {
                if let Some(id) = receipt.get("id") {
                    capture["id"] = id.clone();
                }
            }
            if self.listen_ui.capture.as_ref().is_some_and(|previous| {
                previous["job"] == capture["job"] && previous["cancel_requested"] == true
            }) && listen_capture_active(&capture)
            {
                capture["cancel_requested"] = json!(true);
            }
            self.listen_ui.capture = Some(capture);
            self.listen_ui.error = None;
        }
    }

    pub(crate) fn listen_command_error(&mut self, command: &str, args: &Value, error: String) {
        if command == "inspect_listen" {
            if let Some(id) = args["id"].as_str() {
                self.listen_ui.statuses.insert(
                    id.to_owned(),
                    json!({"id":id,"available":false,"reason":error}),
                );
            }
        } else {
            self.listen_ui.error = Some(error);
        }
    }

    pub(crate) fn listen_inspector(
        &mut self,
        ctx: &egui::Context,
        revision: u64,
    ) -> Option<RackAction> {
        let id = self.listen_ui.inspected.clone()?;
        let mut action = None;
        let mut close = false;
        egui::SidePanel::right("listen-capture-inspector").default_width(330.0).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Listen capture");
                if ui.small_button("Close").clicked() { close = true; }
            });
            ui.label(format!("Module · {id}"));
            if let Some(status) = self.listen_ui.statuses.get(&id) {
                if status["available"] == true {
                    let source = &status["listen"];
                    ui.label(format!("Source frames · {}", source["frames_seen"]));
                    ui.label(format!("Source dropped · {}", source["dropped"]));
                } else {
                    ui.label(status["reason"].as_str().unwrap_or("Live observation unavailable"));
                }
            } else { ui.label("Live observation unavailable"); }
            ui.separator();
            ui.label("Outputs retain independent raw L/R channels. Capture sums each input's polyphonic voices. Absent right remains zero; WAV export alone duplicates left to right.");
            if let Some(error) = &self.listen_ui.error { ui.colored_label(Color32::from_rgb(220, 117, 99), error); }
            if let Some(capture) = self.listen_ui.capture.as_ref().filter(|capture| capture["id"] == id || capture.get("id").is_none()) {
                ui.separator();
                ui.label(format!("{} · {}", capture["job"].as_str().unwrap_or("Capture"), capture["state"].as_str().unwrap_or("unavailable")));
                ui.label(format!("Phase · {}", capture["phase"].as_str().unwrap_or("unavailable")));
                let collected = capture["collected_frames"].as_u64().unwrap_or(0);
                let target = capture["target_frames"].as_u64().unwrap_or(0);
                if target > 0 {
                    ui.add(egui::ProgressBar::new((collected as f32 / target as f32).min(1.0)).text(format!("{collected} / {target} frames")));
                }
                if capture["cancel_requested"] == true { ui.label("Cancellation requested · awaiting worker"); }
                if listen_capture_active(capture) && ui.add_enabled(capture["cancel_requested"] != true, egui::Button::new("Cancel capture")).clicked() {
                    action = Some(RackAction::Command("cancel_listen_capture", json!({"job":capture["job"],"expected_revision":revision})));
                }
                if let Some(result) = capture.get("result").filter(|result| !result.is_null()) {
                    ui.separator();
                    ui.label("Capture result");
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(error) = result["error"].as_str() { ui.colored_label(Color32::from_rgb(220, 117, 99), error); }
                        if let Some(analysis) = result.get("analysis") {
                            for row in listen_analysis_rows(analysis) { ui.label(row); }
                        }
                        if let Some(path) = result["wav_file"].as_str() { ui.label(format!("WAV saved · {path}")); }
                        if let Some(error) = result["wav_error"].as_str() { ui.label(format!("WAV error · {error}")); }
                        ui.collapsing("Measured details and spectrum", |ui| {
                            ui.add(egui::Label::new(serde_json::to_string_pretty(result).unwrap_or_default()).wrap());
                        });
                    });
                }
            } else { ui.label("Choose a duration and start Capture on the module."); }
        });
        if close {
            self.listen_ui.inspected = None;
        }
        action
    }

    fn atlas_listen_panel(
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
        let text = |x, y, label: &str, size, color| {
            painter.text(
                p(x, y),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(painter.ctx(), size * zoom),
                color,
            );
        };
        let status = self.listen_ui.statuses.get(&node.id);
        let actual = status
            .filter(|status| status["available"] == true)
            .and_then(|status| status.get("listen"));
        let available = actual.is_some();
        let capturing = actual.is_some_and(|source| source["capturing"] == true);
        let capture_strength = if capturing { 1.0 } else { 0.0 };
        painter.circle_filled(p(25.0, 74.0), 5.4 * zoom, Color32::from_gray(58));
        self.lighting.circle(
            &painter,
            p(25.0, 74.0),
            2.2 * zoom,
            Color32::from_rgb(235, 65, 48).gamma_multiply(0.12 + 0.88 * capture_strength),
            capture_strength,
            0.12,
        );
        text(
            182.0,
            74.0,
            if capturing {
                "CAPTURING"
            } else if available {
                "READY"
            } else {
                "WAITING FOR AUDIO"
            },
            9.0,
            INK,
        );
        for x in [70.0, 290.0] {
            painter.arrow(
                p(x, 151.0),
                vec2(0.0, 27.0) * zoom,
                Stroke::new(zoom, MUTED),
            );
        }
        text(180.0, 151.0, "RAW L / R", 11.0, INK);
        text(180.0, 173.0, "INDEPENDENT", 8.0, MUTED);
        text(
            180.0,
            247.0,
            &actual.map_or_else(
                || "INPUT STATUS UNAVAILABLE".to_owned(),
                |source| {
                    format!(
                        "L {} · R {}",
                        if source["left_connected"] == true {
                            "CONNECTED"
                        } else {
                            "ABSENT"
                        },
                        if source["right_connected"] == true {
                            "CONNECTED"
                        } else {
                            "ABSENT"
                        }
                    )
                },
            ),
            9.0,
            INK,
        );
        let global_active = self
            .listen_ui
            .capture
            .as_ref()
            .is_some_and(listen_capture_active);
        let active = self
            .listen_ui
            .capture
            .as_ref()
            .is_some_and(|capture| listen_capture_active(capture) && capture["id"] == node.id);
        let labels = [
            ("duration", format!("{:.1} S  ▾", self.listen_ui.seconds)),
            (
                "wav",
                format!(
                    "{} WRITE WAV",
                    if self.listen_ui.write_wav {
                        "✓"
                    } else {
                        "□"
                    }
                ),
            ),
            (
                "capture",
                if active {
                    "CANCEL".to_owned()
                } else if global_active {
                    "BUSY".to_owned()
                } else {
                    "CAPTURE".to_owned()
                },
            ),
            ("inspect", "INSPECT".to_owned()),
        ];
        let mut action = None;
        for (name, label) in labels {
            let local = atlas_listen_control(name);
            let bounds =
                Rect::from_min_max(p(local.min.x, local.min.y), p(local.max.x, local.max.y));
            let enabled =
                name != "capture" || (available && !node.bypassed && !global_active) || active;
            let response = ui.interact(
                bounds,
                ui.id().with((&node.id, "listen", name)),
                if interactive && enabled {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            painter.rect_filled(bounds, 3.0 * zoom, Color32::from_rgb(222, 211, 180));
            painter.rect_stroke(
                bounds,
                3.0 * zoom,
                Stroke::new(zoom, MUTED),
                egui::StrokeKind::Inside,
            );
            text(
                local.center().x,
                local.center().y,
                &label,
                11.0,
                if enabled { INK } else { MUTED },
            );
            if response.clicked() {
                match name {
                    "duration" => {
                        self.listen_ui.duration_menu = (self.listen_ui.duration_menu.as_deref()
                            != Some(&node.id))
                        .then(|| node.id.clone())
                    }
                    "wav" => self.listen_ui.write_wav = !self.listen_ui.write_wav,
                    "capture" => {
                        self.listen_ui.inspected = Some(node.id.clone());
                        action = Some(
                            if let Some(capture) = self
                                .listen_ui
                                .capture
                                .as_ref()
                                .filter(|capture| listen_capture_active(capture))
                            {
                                RackAction::Command(
                                    "cancel_listen_capture",
                                    json!({"job":capture["job"],"expected_revision":revision}),
                                )
                            } else {
                                RackAction::Command(
                                    "start_listen_capture",
                                    json!({"id":node.id,"seconds":self.listen_ui.seconds,"write_wav":self.listen_ui.write_wav,"expected_revision":revision}),
                                )
                            },
                        );
                    }
                    _ => {
                        self.listen_ui.inspected = Some(node.id.clone());
                        action = Some(RackAction::Command(
                            "inspect_listen",
                            json!({"id":node.id,"expected_revision":revision}),
                        ));
                    }
                }
            }
        }
        text(
            180.0,
            377.0,
            "RECORDS SUMMED VOICES · AUDIO PASSES THROUGH",
            8.5,
            INK,
        );
        if interactive && self.listen_ui.duration_menu.as_deref() == Some(&node.id) {
            // A compact anchored dropdown, with no modal dialog or file picker.
            egui::Area::new(ui.id().with((&node.id, "listen-duration")))
                .order(egui::Order::Foreground)
                .fixed_pos(p(24.0, 307.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        for seconds in [0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0] {
                            if ui
                                .selectable_label(
                                    self.listen_ui.seconds == seconds,
                                    format!("{seconds:.1} s"),
                                )
                                .clicked()
                            {
                                self.listen_ui.seconds = seconds;
                                self.listen_ui.duration_menu = None;
                            }
                        }
                    });
                });
        }
        action
    }
}
