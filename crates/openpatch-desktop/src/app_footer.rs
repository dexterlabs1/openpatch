impl OpenPatch {
    fn footer(&mut self, ctx: &egui::Context) {
        let timing = self.diagnostic_snapshot();
        self.audio_timing_history
            .observe(timing.as_ref(), ctx.input(|input| input.time));
        let timing_label = timing_footer(timing.as_ref());
        let timing_help = timing_explanation(timing_status(timing.as_ref()));
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(CHROME).inner_margin(egui::Margin::symmetric(12,5)))
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Audio health stays visible even when a status message is long.
                    let timing_button = ui.button(RichText::new(&timing_label).size(10.0).color(ACCENT))
                        .on_hover_text(format!("{timing_help}\nOpen Audio performance for details."));
                    ctx.data_mut(|data| data.insert_temp(egui::Id::new("audio-performance-anchor"), (timing_button.rect, timing_button.layer_id)));
                    if timing_button.clicked() {
                        self.audio_performance_open = !self.audio_performance_open;
                    }
                    self.footer_room_brightness(ui);
                    if self.audio.is_none() {
                        if ui.button("Retry audio").on_hover_text(&self.device).clicked() {
                            self.reopen_audio(self.output_device.clone());
                        }
                        ui.label(RichText::new("No audio output").size(10.0).color(MUTED)).on_hover_text(&self.device);
                    } else if let Some(audio)=&self.audio {
                        if ctx.content_rect().width() >= 1050.0 {
                        ui.label(RichText::new(format!("{} Hz",audio.sample_rate)).size(10.0).color(MUTED)).on_hover_text(&self.device);
                        ui.add(egui::ProgressBar::new(audio.peak().min(1.0)).desired_width(48.0).desired_height(5.0).fill(ACCENT))
                            .on_hover_text("Combined output peak · after master volume and graph crossfade");
                        ui.label(RichText::new(if audio.is_playing(){"Playing"}else{"Paused"}).size(10.0).color(MUTED));
                        if audio.solver_failures()!=0 {
                            ui.colored_label(ACCENT,"Solver holds").on_hover_text(format!("{} circuit solves reached the bounded work limit. The last valid capacitor state was retained. Reduce drive, resonance, or modulation.",audio.solver_failures()));
                        }
                    }
                        }
                    if !self.mcp_session.is_idle() {
                        self.mcp_status(ui);
                    }
                    ui.separator();
                    ui.allocate_ui_with_layout(ui.available_size(),egui::Layout::left_to_right(egui::Align::Center),|ui| {
                        if let Some(action) = self.construction.footer(ui) {
                            self.construction_footer_action = Some(action);
                        }
                        ui.label(RichText::new(format!("{} modules · {} cables",self.patch.nodes.len(),self.patch.cables.len())).size(10.0).color(MUTED))
                            .on_hover_text(if self.patch.version==2 {"Polyphonic engine v2"}else{"Legacy mono engine"});
                        if self.rack.selection_count()>1 {
                            ui.label(RichText::new(format!("{} selected",self.rack.selection_count())).size(10.0).color(ACCENT))
                                .on_hover_text("Shift-click headers to select modules. Cmd/Ctrl+D duplicates the selection; add Shift to include incoming cables.");
                        }
                        ui.separator();
                        ui.add(egui::Label::new(RichText::new(&self.status).size(10.0).color(ACCENT)).truncate()).on_hover_text(&self.status);
                    });
                });
            });
    }
    fn footer_room_brightness(&mut self, ui: &mut egui::Ui) {
        let previous = self.rack.room_brightness();
        let mut percent = previous * 100.0;
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = 84.0;
            let label = format!("Room {percent:.0}%");
            ui.label(RichText::new(label).size(10.0).color(MUTED));
            let response = ui
                .add(
                    egui::Slider::new(&mut percent, 0.0..=100.0)
                        .show_value(false)
                        .step_by(0.1),
                )
                .on_hover_text(
                    "Room brightness · dims panel surfaces while module lights keep emitting",
                );
            let changed = response.changed() && (percent / 100.0 - previous).abs() > f32::EPSILON;
            let key = egui::Id::new("footer-room-brightness-dirty");
            if changed {
                self.rack.set_room_brightness(percent / 100.0);
                ui.ctx().data_mut(|data| data.insert_temp(key, true));
            }
            if (response.drag_stopped() || (changed && !response.dragged()))
                && ui
                    .ctx()
                    .data_mut(|data| data.remove_temp::<bool>(key))
                    .unwrap_or(false)
            {
                self.save_host_settings();
            }
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("footer-room-brightness-response"), response)
            });
        });
    }
}
