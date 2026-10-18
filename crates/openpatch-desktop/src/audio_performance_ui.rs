// UI-only observations of coherent callback publications. No audio-thread storage.
#[derive(Default)]
struct AudioTimingHistory {
    points: std::collections::VecDeque<(f64, f64)>,
    last: Option<(u32, u64, u64)>,
}
impl AudioTimingHistory {
    const LIMIT: usize = 180;

    fn observe(&mut self, snapshot: Option<&AudioTimingSnapshot>, time: f64) {
        let Some(snapshot) = snapshot else {
            self.clear();
            return;
        };
        if snapshot.closed {
            self.clear();
            return;
        }
        if !snapshot.coherent || !time.is_finite() {
            return;
        }
        if self.last.is_some_and(|(rate, epoch, sequence)| {
            rate != snapshot.sample_rate
                || epoch != snapshot.epoch
                || sequence > snapshot.callback_sequence
        }) || self
            .points
            .back()
            .is_some_and(|(previous, _)| time < *previous)
        {
            self.clear();
        }
        if !snapshot.available || snapshot.reset_pending {
            return;
        }
        let Some(value) = snapshot
            .utilization
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            return;
        };
        let identity = (
            snapshot.sample_rate,
            snapshot.epoch,
            snapshot.callback_sequence,
        );
        if self.last == Some(identity) {
            return;
        }
        if self
            .points
            .back()
            .is_some_and(|(previous, _)| time - *previous < 0.1)
        {
            return;
        }
        self.last = Some(identity);
        self.points.push_back((time, value));
        while self.points.len() > Self::LIMIT {
            self.points.pop_front();
        }
    }

    fn clear(&mut self) {
        self.points.clear();
        self.last = None;
    }

    fn graph(&self, ui: &mut egui::Ui) {
        ui.label(RichText::new("DSP load history").strong());
        ui.small("Recent measured callback budget use · up to 18 seconds");
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 112.0),
            egui::Sense::hover(),
        );
        let plot = rect.shrink2(egui::vec2(6.0, 12.0));
        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, Color32::from_rgb(19, 22, 27));
        if self.points.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Waiting for measured callbacks",
                egui::FontId::proportional(11.0),
                MUTED,
            );
        } else {
            let ceiling = self
                .points
                .iter()
                .map(|(_, value)| *value)
                .fold(1.25_f64, f64::max)
                .min(4.0);
            let end = self.points.back().unwrap().0;
            let span = (end - self.points.front().unwrap().0).clamp(1.0, 18.0);
            let deadline = plot.bottom() - plot.height() / ceiling as f32;
            painter.line_segment(
                [
                    egui::pos2(plot.left(), deadline),
                    egui::pos2(plot.right(), deadline),
                ],
                egui::Stroke::new(1.0_f32, Color32::from_rgb(131, 85, 53)),
            );
            painter.text(
                egui::pos2(plot.left() + 3.0, deadline - 1.0),
                egui::Align2::LEFT_BOTTOM,
                "100% deadline",
                egui::FontId::proportional(9.0),
                MUTED,
            );
            let points: Vec<_> = self
                .points
                .iter()
                .filter(|(time, _)| *time >= end - span)
                .map(|(time, value)| {
                    egui::pos2(
                        plot.right() - ((end - *time) / span) as f32 * plot.width(),
                        plot.bottom() - (*value / ceiling).clamp(0.0, 1.0) as f32 * plot.height(),
                    )
                })
                .collect();
            for segment in points.windows(2) {
                painter.line_segment([segment[0], segment[1]], egui::Stroke::new(1.8_f32, ACCENT));
            }
            if let Some(point) = points.last() {
                painter.circle_filled(*point, 2.5, ACCENT);
            }
        }
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("audio-performance-graph-rect"), rect));
    }
}

impl OpenPatch {
    fn audio_performance(&mut self, ctx: &egui::Context) {
        if !self.audio_performance_open {
            return;
        }
        let snapshot = self.diagnostic_snapshot();
        let status = timing_status(snapshot.as_ref());
        let simulated = self.smoke_test.is_some() && self.smoke_diagnostics.is_some();
        let mut open = true;
        let mut reset = false;
        let (anchor, layer) = ctx
            .data(|data| {
                data.get_temp::<(egui::Rect, egui::LayerId)>(egui::Id::new(
                    "audio-performance-anchor",
                ))
            })
            .unwrap_or((
                egui::Rect::from_min_size(
                    ctx.content_rect().right_bottom() - egui::vec2(100.0, 30.0),
                    egui::vec2(88.0, 24.0),
                ),
                egui::LayerId::background(),
            ));
        let width = 440.0_f32.min(ctx.content_rect().width() - 28.0).max(240.0);
        let response = egui::Popup::new(egui::Id::new("audio-performance-dropdown"), ctx.clone(), anchor, layer)
            .kind(egui::PopupKind::Menu)
            .open_bool(&mut open)
            .align(egui::emath::RectAlign::TOP_END)
            .align_alternatives(&[])
            .gap(6.0)
            .width(width)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(width);
                egui::ScrollArea::vertical().id_salt("audio-performance-scroll")
                    .max_height((anchor.top() - ctx.content_rect().top() - 28.0).max(80.0))
                    .show(ui, |ui| {
                ui.heading("Audio performance");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            self.audio.is_some() && !simulated && status != TimingStatus::Stopped,
                            egui::Button::new("Reset statistics"),
                        )
                        .on_hover_text("Start a new measurement interval on the next audio callback")
                        .clicked()
                    {
                        reset = true;
                    }
                    if status == TimingStatus::ResetPending {
                        ui.label("Waiting for callback…");
                    }
                });
                if simulated {
                    ui.colored_label(ACCENT, "SIMULATED DATA · SCREENSHOT FIXTURE");
                } else if self.audio.as_ref().is_some_and(|audio| !audio.is_playing())
                    && status == TimingStatus::Ready
                {
                    ui.label("Transport paused · audio callbacks continue");
                }
                ui.label(timing_explanation(status));
                self.audio_timing_history.graph(ui);
                if status == TimingStatus::Ready {
                    let snapshot = snapshot.as_ref().unwrap();
                    let percent = |value: f64| format!("{:.1}%", value * 100.0);
                    let current = snapshot.utilization.map(percent).unwrap_or_else(|| "—".into());
                    let peak = snapshot.peak_utilization.map(percent).unwrap_or_else(|| "—".into());
                    let mean = weighted_mean_utilization(snapshot)
                        .map(percent)
                        .unwrap_or_else(|| "—".into());
                    ui.separator();
                    egui::Grid::new("audio-performance-metrics")
                        .num_columns(2)
                        .spacing([18.0, 5.0])
                        .show(ui, |ui| {
                            ui.label("Current budget use");
                            ui.strong(current);
                            ui.end_row();
                            ui.label("Peak budget use");
                            ui.strong(peak);
                            ui.end_row();
                            ui.label("Weighted mean");
                            ui.strong(mean);
                            ui.end_row();
                            ui.label("Processing budget overruns");
                            ui.colored_label(ACCENT, snapshot.overruns.to_string());
                            ui.end_row();
                            ui.label("Last processing / budget");
                            ui.label(format!(
                                "{:.3} / {:.3} ms",
                                snapshot.duration_ns as f64 / 1_000_000.0,
                                snapshot.frames as f64 * 1000.0 / snapshot.sample_rate as f64
                            ));
                            ui.end_row();
                            ui.label("Maximum processing time");
                            ui.label(format!("{:.3} ms", snapshot.max_duration_ns as f64 / 1_000_000.0));
                            ui.end_row();
                            ui.label("Last deadline headroom");
                            ui.label(format!("{:.3} ms", snapshot.frames as f64 * 1000.0 / snapshot.sample_rate as f64 - snapshot.duration_ns as f64 / 1_000_000.0));
                            ui.end_row();
                            ui.label("Observed audio interval");
                            ui.label(format!("{:.2} s", snapshot.total_frames as f64 / snapshot.sample_rate as f64));
                            ui.end_row();
                            ui.label("Measured callbacks");
                            ui.label(snapshot.callbacks.to_string());
                            ui.end_row();
                            ui.label("Invalid or empty callbacks");
                            ui.label(snapshot.invalid_callbacks.to_string());
                            ui.end_row();

                            ui.label("Graph transitions in interval");
                            ui.label(snapshot.transition_callbacks.to_string());
                            ui.end_row();
                        });
                    ui.separator();
                    ui.label(format!(
                        "Last callback · {} frames × {} channels at {} Hz",
                        snapshot.frames, snapshot.channels, snapshot.sample_rate
                    ));
                    ui.label(format!(
                        "Graph revision {} · callback sequence {}",
                        snapshot.revision, snapshot.callback_sequence
                    ));
                    ui.label(format!(
                        "Graph adopted: {} · crossfade: {}",
                        if snapshot.adopted_graph { "yes" } else { "no" },
                        if snapshot.crossfade { "yes" } else { "no" }
                    ));
                }
                ui.separator();
                ui.small("Hardware underruns / xruns · not reported by this backend");
                ui.label(RichText::new("Processing overruns mean measured callback time exceeded its frame budget. Elapsed time includes preemption during rendering; it does not measure late callback delivery or hardware xruns.").size(10.0).color(MUTED));

                });
            });
        if let Some(response) = response {
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("audio-performance-dropdown-rect"),
                    response.response.rect,
                )
            });
        }
        self.audio_performance_open = open;
        if open {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if reset {
            if let Some(audio) = &self.audio {
                audio.reset_diagnostics();
                self.audio_timing_history.clear();
                ctx.request_repaint();
            }
        }
    }
}
