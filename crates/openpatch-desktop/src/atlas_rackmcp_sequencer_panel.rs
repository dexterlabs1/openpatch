// RackMCP's saved patterns and target lanes have their own panel and identity.
// Authored cells never imply a published live position or gate observation.
fn atlas_rackmcp_control(name: &str) -> (Pos2, f32) {
    let x = match name {
        "bpm" => 60.0,
        "swing" => 150.0,
        "running" => 240.0,
        _ => 330.0,
    };
    (pos2(x, 114.0), 28.0)
}

fn atlas_rackmcp_value_center(name: &str) -> Pos2 {
    pos2(atlas_rackmcp_control(name).0.x, 157.0)
}

fn atlas_rackmcp_port(input: bool, index: usize) -> Pos2 {
    if input {
        pos2(60.0 + index as f32 * 80.0, 373.0)
    } else if index < 16 {
        pos2(
            414.0 + (index % 4) as f32 * 84.0,
            129.0 + (index / 4) as f32 * 74.0,
        )
    } else {
        pos2(230.0 + (index - 16) as f32 * 80.0, 373.0)
    }
}

fn paint_atlas_rackmcp_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    painter.text(
        rect.min + vec2(atlas_rackmcp_control(name).0.x, 77.0) * zoom,
        Align2::CENTER_CENTER,
        match name {
            "bpm" => "TEMPO · BPM",
            "swing" => "SWING · 50–75%",
            "running" => "RUN",
            _ => "RESET",
        },
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}

fn paint_atlas_rackmcp_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
    live: Option<&Value>,
) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let text = |x, y, value: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let bounded = |y, value: String, size, color| {
        let mut job = egui::text::LayoutJob::simple(
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
            338.0 * zoom,
        );
        job.wrap.max_rows = 1;
        let galley = painter.layout_job(job);
        painter.galley(p(190.0, y) - galley.size() * 0.5, galley, color);
    };
    text(
        190.0,
        55.0,
        &live.map_or_else(
            || "FOUR TRACKS · UP TO FOUR VOICES EACH".to_owned(),
            |live| {
                format!(
                    "{} · {:.1} BPM · {} CLOCK",
                    if live["running"] == true {
                        "RUNNING"
                    } else {
                        "STOPPED"
                    },
                    live["bpm"].as_f64().unwrap_or(0.0),
                    if live["external_clock"] == true {
                        "EXT"
                    } else {
                        "INT"
                    }
                )
            },
        ),
        8.5,
        MUTED,
    );
    text(540.0, 55.0, "PITCH = NOTES / 12 V · 0 V = C4", 9.0, INK);
    for track in 0..4 {
        text(
            414.0 + track as f32 * 84.0,
            77.0,
            &format!("TRACK {}", track + 1),
            10.0,
            INK,
        );
    }
    painter.line_segment(
        [p(378.0, 65.0), p(378.0, 389.0)],
        Stroke::new(zoom, Color32::from_rgb(153, 141, 111)),
    );

    let state = node.rackmcp_sequence.as_ref();
    let data = state.map(|state| &state.data);
    let selected = data
        .and_then(|data| {
            data.selected_pattern.as_ref().and_then(|name| {
                data.patterns
                    .iter()
                    .position(|pattern| &pattern.name == name)
            })
        })
        .unwrap_or(0);
    let pattern = data.and_then(|data| {
        live.and_then(|live| live["active_pattern_name"].as_str())
            .and_then(|name| data.patterns.iter().find(|pattern| pattern.name == name))
            .or_else(|| data.patterns.get(selected))
    });
    let length = pattern.map_or(16, |pattern| pattern.length).clamp(1, 64);
    let division = pattern.map_or(4, |pattern| pattern.steps_per_beat);
    let start = crate::lighting::next(painter);
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width: 720.0 },
        zoom,
    };
    draw.screen(
        Rect::from_min_max(p(20.0, 173.0), p(360.0, 300.0)),
        lighting,
    );
    let amber = Color32::from_rgb(229, 183, 103);
    let hit = Color32::from_rgb(137, 174, 137);
    bounded(
        184.0,
        live.map_or_else(
            || {
                format!(
                    "AUTHORED · {} · {length} STEPS · {division} / BEAT",
                    pattern.map_or("A", |pattern| pattern.name.as_str())
                )
            },
            |live| {
                format!(
                    "ACTIVE {} · NEXT {}",
                    live["active_pattern_name"].as_str().unwrap_or("—"),
                    live["pending_pattern_name"].as_str().unwrap_or("—")
                )
            },
        ),
        9.0,
        amber,
    );
    for track in 0..4 {
        let y = 221.0 + track as f32 * 20.0;
        let authored = pattern.and_then(|pattern| pattern.tracks.get(track));
        text(
            48.0,
            y,
            &format!(
                "T{} · {} VOICES",
                track + 1,
                authored.map_or(1, |track| track.voices)
            ),
            8.0,
            amber,
        );
        let mut steps = [None; 64];
        if let Some(track) = authored {
            for step in &track.steps {
                if let Some(slot) = steps.get_mut(usize::from(step.step)) {
                    *slot = Some(step);
                }
            }
        }
        let width = 267.0 / f32::from(length);
        for (index, step) in steps.iter().enumerate().take(usize::from(length)) {
            let cell = Rect::from_min_max(
                p(81.0 + index as f32 * width, y - 4.0),
                p(81.0 + (index + 1) as f32 * width - 0.7, y + 4.0),
            );
            // Empty notes are source drum hits with held pitch, not rests.
            let enabled = step.is_some_and(|step| step.prob > 0.0);
            let fill = if enabled {
                if step.is_some_and(|step| step.notes.is_empty()) {
                    hit
                } else {
                    amber
                }
            } else {
                Color32::from_rgb(44, 53, 45)
            };
            painter.rect_filled(cell, 0.0, fill);
            if step.is_some() && !enabled {
                painter.rect_stroke(
                    cell,
                    0.0,
                    Stroke::new(0.6 * zoom, Color32::from_rgb(124, 125, 101)),
                    egui::StrokeKind::Inside,
                );
            }
            if enabled && step.is_some_and(|step| step.accent) {
                painter.circle_filled(
                    cell.center() - vec2(0.0, 6.5) * zoom,
                    1.4 * zoom,
                    Color32::from_rgb(250, 234, 190),
                );
            }
        }
    }
    if let Some(live) = live {
        // A playhead is drawn only over the exact pattern reported by the callback.
        if pattern.is_some_and(|pattern| {
            live["active_pattern_name"].as_str() == Some(pattern.name.as_str())
        }) {
            if let Some(step) = live["step"]
                .as_i64()
                .filter(|step| (0..i64::from(length)).contains(step))
            {
                let width = 267.0 / f32::from(length);
                let cell = Rect::from_min_max(
                    p(81.0 + step as f32 * width, 214.0),
                    p(81.0 + (step + 1) as f32 * width - 0.7, 287.0),
                );
                painter.rect_stroke(
                    cell,
                    1.0 * zoom,
                    Stroke::new(1.4 * zoom, Color32::from_rgb(190, 225, 193)),
                    egui::StrokeKind::Outside,
                );
            }
        }
        text(
            190.0,
            293.0,
            &format!(
                "BAR {:.2} · BEAT {:.2} · STEP {} · LOOP {}",
                live["bar"].as_f64().unwrap_or(0.0),
                live["beat"].as_f64().unwrap_or(0.0),
                live["step"].as_i64().map_or_else(
                    || "—".to_owned(),
                    |step| if step < 0 {
                        "—".to_owned()
                    } else {
                        (step + 1).to_string()
                    }
                ),
                live["loop_count"]
            ),
            8.0,
            amber,
        );
    } else {
        text(
            190.0,
            293.0,
            "SAVED NOTES · LIVE POSITION UNAVAILABLE",
            8.0,
            amber,
        );
    }
    lighting.preserve(painter, start);
    let lanes = data.map_or(&[][..], |data| data.lanes.as_slice());
    let targets = state.map_or(&[][..], |state| state.targets.as_slice());
    let native = lanes
        .iter()
        .filter(|lane| {
            targets.iter().any(|target| {
                target.source_module_id == lane.module_id && target.source_param_id == lane.param_id
            })
        })
        .count();
    bounded(
        311.0,
        format!("AUTOMATION · {} / 8 LANES · {native} NATIVE", lanes.len()),
        8.5,
        INK,
    );
    let sources = if lanes.is_empty() {
        "SOURCE TARGETS · NONE".to_owned()
    } else {
        format!(
            "SOURCE TARGETS · {}",
            lanes
                .iter()
                .map(|lane| format!("{}:{}", lane.module_id, lane.param_id))
                .collect::<Vec<_>>()
                .join(" · ")
        )
    };
    bounded(326.0, sources, 8.0, MUTED);
    text(
        190.0,
        340.0,
        "AMBER NOTES · GREEN HITS · DOT ACCENT",
        8.0,
        MUTED,
    );
    text(
        540.0,
        383.0,
        "GATE / ACCENT 10 V · VELOCITY × 10 V",
        9.0,
        MUTED,
    );
}

#[derive(Default)]
struct RackMcpUi {
    statuses: BTreeMap<String, (Value, std::time::Instant)>,
    next_poll: Option<std::time::Instant>,
    menu: Option<(String, u64, &'static str)>,
    select_now: BTreeMap<String, bool>,
    seek_bars: BTreeMap<String, f64>,
    errors: BTreeMap<String, String>,
}

fn atlas_rackmcp_native_control(name: &str) -> Rect {
    let (left, right) = match name {
        "pattern" => (28.0, 205.0),
        "quantize" => (211.0, 276.0),
        _ => (282.0, 352.0),
    };
    Rect::from_min_max(pos2(left, 193.0), pos2(right, 211.0))
}

impl Rack {
    pub(crate) fn rackmcp_poll_commands(
        &mut self,
        patch: &Patch,
        revision: u64,
    ) -> Vec<(&'static str, Value)> {
        let present = |id: &str| {
            patch
                .nodes
                .iter()
                .any(|n| n.id == id && n.kind == "rackmcp_sequencer")
        };
        self.rackmcp_ui.statuses.retain(|id, (receipt, _)| {
            present(id) && receipt["revision"].as_u64() == Some(revision)
        });
        self.rackmcp_ui.errors.retain(|id, _| present(id));
        self.rackmcp_ui.select_now.retain(|id, _| present(id));
        self.rackmcp_ui.seek_bars.retain(|id, _| present(id));
        if self
            .rackmcp_ui
            .menu
            .as_ref()
            .is_some_and(|(id, opened, _)| !present(id) || *opened != revision)
        {
            self.rackmcp_ui.menu = None;
        }
        let now = std::time::Instant::now();
        if self.rackmcp_ui.next_poll.is_some_and(|next| now < next) {
            return Vec::new();
        }
        self.rackmcp_ui.next_poll = Some(now + std::time::Duration::from_millis(100));
        patch
            .nodes
            .iter()
            .filter(|n| n.kind == "rackmcp_sequencer")
            .map(|n| {
                (
                    "inspect_rackmcp_sequence",
                    json!({"id":n.id,"expected_revision":revision}),
                )
            })
            .collect()
    }

    pub(crate) fn accept_rackmcp_receipt(&mut self, name: &str, receipt: Value) {
        if name == "inspect_rackmcp_sequence" {
            if let Some(id) = receipt["id"].as_str() {
                self.rackmcp_ui
                    .statuses
                    .insert(id.to_owned(), (receipt, std::time::Instant::now()));
            }
        }
        // A transport acknowledgment is not an observed position or selection.
        if name == "rackmcp_transport" {
            self.rackmcp_ui.next_poll = None;
        }
    }

    pub(crate) fn rackmcp_command_error(&mut self, name: &str, args: &Value, error: String) {
        if let Some(id) = args["id"].as_str() {
            if name == "inspect_rackmcp_sequence" {
                self.rackmcp_ui.statuses.remove(id);
            }
            self.rackmcp_ui.errors.insert(id.to_owned(), error);
        }
    }

    fn atlas_rackmcp_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let p = |point: Pos2| rect.min + point.to_vec2() * zoom;
        // Catalog previews always show saved notation, even if the renderer has
        // previously seen a live module with the same ID.
        let receipt = interactive
            .then(|| self.rackmcp_ui.statuses.get(&node.id))
            .flatten();
        let actual = receipt
            .filter(|(receipt, received)| {
                receipt["available"] == true
                    && receipt["revision"].as_u64() == Some(revision)
                    && received.elapsed() < std::time::Duration::from_millis(500)
            })
            .and_then(|(receipt, _)| receipt.get("rackmcp_sequence"))
            .filter(|live| live.is_object())
            .cloned();
        let live = actual.as_ref();
        paint_atlas_rackmcp_configuration(&painter, node, rect, zoom, &self.lighting, live);
        let available = live.is_some() && !node.bypassed;
        let selected = live
            .and_then(|live| live["selected_pattern_name"].as_str())
            .or_else(|| {
                node.rackmcp_sequence
                    .as_ref()
                    .and_then(|state| state.data.selected_pattern.as_deref())
            })
            .unwrap_or("A");
        let now = self
            .rackmcp_ui
            .select_now
            .get(&node.id)
            .copied()
            .unwrap_or(false);
        let unavailable = self
            .rackmcp_ui
            .errors
            .get(&node.id)
            .cloned()
            .or_else(|| {
                receipt.and_then(|(receipt, _)| receipt["reason"].as_str().map(str::to_owned))
            })
            .unwrap_or_else(|| "Live observation unavailable".into());
        let mut action = None;
        for (name, label) in [
            ("pattern", format!("SELECT {selected}  ▾")),
            ("quantize", if now { "NOW ▾" } else { "WRAP ▾" }.to_owned()),
            ("seek", "SEEK ▾".to_owned()),
        ] {
            let local = atlas_rackmcp_native_control(name);
            let bounds = Rect::from_min_max(p(local.min), p(local.max));
            let response = ui.interact(
                bounds,
                ui.id().with((&node.id, "rackmcp-live", name)),
                if interactive && available {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            painter.rect_filled(bounds, 2.0 * zoom, Color32::from_rgb(43, 51, 43));
            painter.rect_stroke(
                bounds,
                2.0 * zoom,
                Stroke::new(0.7 * zoom, Color32::from_rgb(115, 126, 100)),
                egui::StrokeKind::Inside,
            );
            let mut job = egui::text::LayoutJob::simple(
                label,
                crate::panel_skin::print_font(painter.ctx(), 10.5 * zoom),
                if available {
                    Color32::from_rgb(229, 183, 103)
                } else {
                    Color32::from_rgb(149, 146, 119)
                },
                bounds.width() - 8.0 * zoom,
            );
            job.wrap.max_rows = 1;
            let galley = painter.layout_job(job);
            painter.galley(
                bounds.center() - galley.size() * 0.5,
                galley,
                Color32::WHITE,
            );
            if response.clicked() {
                let target = (node.id.clone(), revision, name);
                self.rackmcp_ui.menu =
                    (self.rackmcp_ui.menu.as_ref() != Some(&target)).then_some(target);
            }
            response.on_hover_text(if available { match name { "pattern" => "Select a saved pattern through live transport; NEXT changes only when audio observes it.", "quantize" => "NOW applies on the next advance; WRAP waits for the pattern boundary.", _ => "Seek the source timeline in bars (zero is the start)." } } else { &unavailable });
        }
        if interactive && available {
            if let Some((id, opened_revision, name)) = self
                .rackmcp_ui
                .menu
                .clone()
                .filter(|(id, opened, _)| id == &node.id && *opened == revision)
            {
                let anchor = atlas_rackmcp_native_control(name);
                let response = egui::Area::new(ui.id().with((&id, "rackmcp-menu"))).order(egui::Order::Foreground).fixed_pos(p(pos2(anchor.left(), anchor.bottom() + 3.0))).show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(235.0);
                        match name {
                            "pattern" => {
                                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                                    let names: Vec<_> = node.rackmcp_sequence.as_ref().map(|state| state.data.patterns.iter().map(|pattern| pattern.name.as_str()).collect()).unwrap_or_else(|| vec!["A"]);
                                    for name in names {
                                        if ui.selectable_label(selected == name, name).clicked() {
                                            self.rackmcp_ui.errors.remove(&id);
                                            action = Some(RackAction::Command("rackmcp_transport", json!({"id":id,"action":"select","pattern":name,"quantize":if now {"now"} else {"wrap"},"expected_revision":opened_revision})));
                                            self.rackmcp_ui.menu = None;
                                        }
                                    }
                                });
                            }
                            "quantize" => for (value, label) in [(false, "Wrap · pattern boundary"), (true, "Now · next advance")] {
                                if ui.selectable_label(now == value, label).clicked() {
                                    self.rackmcp_ui.select_now.insert(id.clone(), value);
                                    self.rackmcp_ui.menu = None;
                                }
                            },
                            _ => {
                                ui.label("Seek to bar · zero is start");
                                let bar = self.rackmcp_ui.seek_bars.entry(id.clone()).or_insert_with(|| live.and_then(|live| live["bar"].as_f64()).unwrap_or(0.0));
                                ui.add(egui::DragValue::new(bar).range(0.0..=100000.0).speed(0.25).max_decimals(3));
                                if ui.button("Seek").clicked() {
                                    self.rackmcp_ui.errors.remove(&id);
                                    action = Some(RackAction::Command("rackmcp_transport", json!({"id":id,"action":"seek","bar":bar,"expected_revision":opened_revision})));
                                    self.rackmcp_ui.menu = None;
                                }
                            }
                        }
                    });
                });
                if ui.input(|input| input.key_pressed(egui::Key::Escape))
                    || (ui.input(|input| input.pointer.any_pressed())
                        && ui
                            .input(|input| input.pointer.interact_pos())
                            .is_some_and(|point| {
                                !response.response.rect.contains(point)
                                    && !Rect::from_min_max(p(anchor.min), p(anchor.max))
                                        .contains(point)
                            }))
                {
                    self.rackmcp_ui.menu = None;
                }
            }
        } else if self
            .rackmcp_ui
            .menu
            .as_ref()
            .is_some_and(|(id, _, _)| id == &node.id)
        {
            self.rackmcp_ui.menu = None;
        }
        action
    }
}
