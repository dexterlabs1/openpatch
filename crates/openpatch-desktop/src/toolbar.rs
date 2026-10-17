// Native toolbar layout and dropdown controls.
impl OpenPatch {
    fn toolbar_menus(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.menu_button("File", |ui| {
            if ui.button("New patch     ⌘N").clicked() {
                self.request_action(PendingAction::NewTemplate);
                ui.close();
            }
            if ui.button("Save template…").clicked() {
                self.begin_save_template();
                ui.close();
            }
            if ui.button("Open…     ⌘O").clicked() {
                self.file_dialog(FileMode::Open);
                ui.close();
            }
            if ui.button("Save     ⌘S").clicked() {
                self.save_current();
                ui.close();
            }
            if ui.button("Save as…     ⇧⌘S").clicked() {
                self.after_save = None;
                self.file_dialog(FileMode::Save);
                ui.close();
            }
            if ui
                .add_enabled(
                    self.active_file.is_some(),
                    egui::Button::new("Revert to saved"),
                )
                .clicked()
            {
                if let Some(name) = self.active_file.clone() {
                    self.request_action(PendingAction::Load(name));
                }
                ui.close();
            }
            if ui
                .add_enabled(
                    !self.recovery_names.is_empty(),
                    egui::Button::new("Recover unsaved work…"),
                )
                .clicked()
            {
                self.recovery_open = true;
                ui.close();
            }
            ui.separator();
            if ui.button("Reference analog voice").clicked() {
                let mut patch = openpatch_core::analog_demo_patch();
                patch.version = 2;
                self.replace_from_menu(patch, "Reference analog voice");
                ui.close();
            }
            if ui.button("Clocked random voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/clocked-random-voice.json"
                ))
                .expect("valid bundled random voice");
                self.replace_from_menu(patch, "Clocked random voice");
                ui.close();
            }
            if ui.button("Synced waveform voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/synced-waveform-voice.json"
                ))
                .expect("valid bundled waveform voice");
                self.replace_from_menu(patch, "Synced waveform voice");
                ui.close();
            }
            if ui.button("Stacked pitch voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stacked-pitch-voice.json"
                ))
                .expect("valid bundled stacked pitch voice");
                self.replace_from_menu(patch, "Stacked pitch voice");
                ui.close();
            }
            if ui.button("Layered stereo morph").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/layered-stereo-morph.json"
                ))
                .expect("valid bundled layered stereo morph");
                self.replace_from_menu(patch, "Layered stereo morph");
                ui.close();
            }
            if ui.button("Delayed gate voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/delayed-gate-voice.json"))
                        .unwrap();
                ui.close();
                self.replace_from_menu(patch, "Delayed gate voice");
            }
            if ui.button("Manual push voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/manual-push-voice.json"))
                        .unwrap();
                ui.close();
                self.replace_from_menu(patch, "Manual push voice");
            }
            if ui.button("Shift register canon").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/shift-register-canon.json"
                ))
                .expect("valid bundled shift register canon");
                ui.close();
                self.replace_from_menu(patch, "Shift register canon");
            }
            if ui.button("Random values voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/random-values-voice.json"
                ))
                .expect("valid bundled random values voice");
                ui.close();
                self.replace_from_menu(patch, "Random values voice");
            }
            if ui.button("Noise colors").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/noise-colors.json"))
                        .expect("valid bundled noise colors");
                ui.close();
                self.replace_from_menu(patch, "Noise colors");
                self.rack.set_zoom(0.85);
            }
            if ui.button("Struck resonator").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/struck-resonator.json"))
                        .expect("valid bundled physical resonator");
                ui.close();
                self.replace_from_menu(patch, "Struck resonator");
                self.rack.set_zoom(0.75);
            }
            if ui.button("Manual pulses voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/manual-pulses-voice.json"
                ))
                .unwrap();
                ui.close();
                self.replace_from_menu(patch, "Manual pulses voice");
            }
            if ui.button("Gate shaping voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/gate-shaping-voice.json"))
                        .expect("valid bundled gate shaping voice");
                self.replace_from_menu(patch, "Gate shaping voice");
                ui.close();
            }
            if ui.button("Annotated drum rack").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/annotated-drum-rack.json"
                ))
                .expect("valid annotated drum rack");
                self.replace_from_menu(patch, "Annotated drum rack");
                ui.close();
            }
            if ui.button("MIDI drum pads").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/midi-gate-drums.json"))
                        .expect("valid MIDI drum pads");
                self.replace_from_menu(patch, "MIDI drum pads");
                ui.close();
            }
            if ui.button("MIDI controller voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/midi-controller-voice.json"
                ))
                .expect("valid MIDI controller voice");
                self.replace_from_menu(patch, "MIDI controller voice");
                ui.close();
            }
            if ui.button("Stock Mix layered voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stock-mix-voice.json"))
                        .expect("valid Stock Mix layered voice");
                self.replace_from_menu(patch, "Stock Mix layered voice");
                ui.close();
            }
            if ui.button("Studio Mixer FX").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/studio-mixer-fx.json"))
                        .expect("valid Studio Mixer FX voice");
                self.replace_from_menu(patch, "Studio Mixer FX");
                ui.close();
            }
            if ui.button("Noise into matter").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stock-noise-resonator.json"
                ))
                .expect("valid Stock Noise resonator");
                self.replace_from_menu(patch, "Noise into matter");
                ui.close();
            }
            if ui.button("Stock Process voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stock-process-voice.json"
                ))
                .expect("valid Stock Process voice");
                self.replace_from_menu(patch, "Stock Process voice");
                ui.close();
            }
            if ui.button("Stock VCA Mix voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stock-vca-mix-voice.json"
                ))
                .expect("valid Stock VCA Mix voice");
                self.replace_from_menu(patch, "Stock VCA Mix voice");
                ui.close();
            }
            if ui.button("Stock Octave voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stock-octave-voice.json"))
                        .expect("valid Stock Octave voice");
                self.replace_from_menu(patch, "Stock Octave voice");
                ui.close();
            }
            if ui.button("Mid/Side stereo voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stock-mid-side-voice.json"
                ))
                .expect("valid Mid/Side stereo voice");
                self.replace_from_menu(patch, "Mid/Side stereo voice");
                ui.close();
            }
            if ui.button("Quantized stock voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stock-quantized-voice.json"
                ))
                .expect("valid Quantized stock voice");
                self.replace_from_menu(patch, "Quantized stock voice");
                ui.close();
            }
            if ui.button("Stock sequenced voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/stock-sequenced-voice.json"
                ))
                .expect("valid Stock sequenced voice");
                self.replace_from_menu(patch, "Stock sequenced voice");
                ui.close();
            }
            if ui.button("Stock modular voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stock-voice.json"))
                        .expect("valid Stock modular voice");
                self.replace_from_menu(patch, "Stock modular voice");
                ui.close();
            }
            if ui.button("Stock VCA voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stock-vca-voice.json"))
                        .expect("valid stock VCA voice");
                self.replace_from_menu(patch, "Stock VCA voice");
                ui.close();
            }
            if ui.button("Random motion voice").clicked() {
                let patch: Patch = serde_json::from_str(include_str!(
                    "../../../examples/random-motion-voice.json"
                ))
                .expect("bundled Random motion voice");
                self.replace_from_menu(patch, "Random motion voice");
                ui.close();
            }
            if ui.button("Wavetable voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/wavetable-voice.json"))
                        .expect("valid wavetable voice");
                self.replace_from_menu(patch, "Wavetable voice");
                ui.close();
            }
            if ui.button("Stock VCF voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stock-vcf-voice.json"))
                        .expect("valid stock VCF voice");
                self.replace_from_menu(patch, "Stock VCF voice");
                ui.close();
            }
            if ui.button("Clocked delay voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stock-delay-voice.json"))
                        .expect("valid clocked delay voice");
                self.replace_from_menu(patch, "Clocked delay voice");
                ui.close();
            }
            if ui.button("Polyphonic ADSR voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/polyphonic-adsr-voice.json"
                ))
                .expect("valid polyphonic ADSR voice");
                self.replace_from_menu(patch, "Polyphonic ADSR voice");
                ui.close();
            }
            if ui.button("Voltage utilities").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/voltage-utilities.json"))
                        .expect("valid bundled voltage utilities");
                self.replace_from_menu(patch, "Voltage utilities");
                ui.close();
            }
            if ui.button("Switched stereo voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/switched-stereo-voice.json"
                ))
                .expect("valid bundled switching voice");
                self.replace_from_menu(patch, "Switched stereo voice");
                ui.close();
            }
            if ui.button("Sequenced analog voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/sequenced-analog-voice.json"
                ))
                .expect("valid bundled sequence");
                self.replace_from_menu(patch, "Sequenced analog voice");
                ui.close();
            }
            if ui.button("Clocked modulation").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/clocked-modulation.json"))
                        .expect("valid bundled modulation voice");
                self.replace_from_menu(patch, "Clocked modulation");
                self.rack.set_zoom(0.75);
                ui.close();
            }
            if ui.button("Stereo width study").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/stereo-width-study.json"))
                        .expect("valid bundled stereo study");
                self.replace_from_menu(patch, "Stereo width study");
                ui.close();
            }
            if ui.button("MIDI keyboard voice").clicked() {
                let patch = serde_json::from_str(include_str!(
                    "../../../examples/midi-keyboard-voice.json"
                ))
                .expect("valid bundled MIDI voice");
                self.replace_from_menu(patch, "MIDI keyboard voice");
                self.keyboard_open = true;
                ui.close();
            }
            if ui.button("Polyphonic voice").clicked() {
                let patch =
                    serde_json::from_str(include_str!("../../../examples/polyphonic-voice.json"))
                        .expect("valid bundled polyphonic voice");
                self.replace_from_menu(patch, "Polyphonic voice");
                ui.close();
            }
            if self.patch.version == 1 && ui.button("Enable polyphony and feedback…").clicked() {
                self.confirm_engine_upgrade = true;
                ui.close();
            }
            if ui.button("Tremolo study").clicked() {
                self.replace_from_menu(openpatch_core::demo_patch(), "Tremolo study");
                ui.close();
            }
        });
        ui.menu_button("Edit", |ui| {
            if ui
                .add_enabled(self.can_undo, egui::Button::new("Undo patch edit   ⌘Z"))
                .clicked()
            {
                self.history(false);
                ui.close();
            }
            if ui
                .add_enabled(self.can_redo, egui::Button::new("Redo patch edit   ⇧⌘Z"))
                .clicked()
            {
                self.history(true);
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    !self.patch.cables.is_empty(),
                    egui::Button::new("Clear cables"),
                )
                .clicked()
            {
                self.command("clear_cables", json!({"expected_revision":self.revision}));
                ui.close();
            }
            if ui.button("Add module…   Enter").clicked() {
                self.browse();
                ui.close();
            }
        });
        let (view, _) = egui::containers::menu::MenuButton::new("View")
            .config(egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside))
            .ui(ui, |ui| {
            let max_height = (ui.ctx().content_rect().bottom() - ui.cursor().min.y - 18.0).max(80.0);
            egui::ScrollArea::vertical().id_salt("view-menu-scroll")
                .max_height(max_height).show(ui, |ui| {
            for (tab, label) in [
                (AnalysisTab::Visuals, "Visualizers"),
                (AnalysisTab::Scope, "Signal scope"),
            ] {
                let mut selected = self.analysis_tab == Some(tab);
                if ui.checkbox(&mut selected, label).changed() {
                    self.select_analysis(selected.then_some(tab));
                }
            }
            ui.checkbox(&mut self.keyboard_open, "Performance keyboard");
            if ui.button("Audio performance…").clicked() {
                self.audio_performance_open = true;
                ui.close();
            }
            ui.separator();
            ui.label("Room brightness");
            let mut percent = self.rack.room_brightness() * 100.0;
            let illumination = ui.add(egui::Slider::new(&mut percent, 0.0..=100.0)
                .suffix("%")
                .text("Room")
                .step_by(0.1))
                .on_hover_text("Dims the rack's reflective surfaces. Live module lights and Scope displays keep emitting light.");
            if illumination.changed() {
                self.rack.set_room_brightness(percent / 100.0);
            }
            // Show every drag frame live, but publish the host
            // preference once at release. Keys/direct entry save
            // immediately because they have no drag lifecycle.
            if illumination.drag_stopped()
                || (illumination.changed() && !illumination.dragged()) {
                self.save_host_settings();
            }
            ui.horizontal(|ui| {
                for (label, amount) in [("Darkroom", 0.1), ("Lights on", 1.0)] {
                    if ui.button(label).clicked() {
                        self.rack.set_room_brightness(amount);
                        self.save_host_settings();
                    }
                }
            });
            ui.separator();
            self.cable_presentation_controls(ui);
            ui.separator();
            ui.menu_button("Motion", |ui| self.agent_motion_controls(ui));
            ui.separator();
            let mut locked = self.rack.positions_locked();
            if ui.checkbox(&mut locked, "Lock module positions").changed() {
                self.rack.set_positions_locked(locked);
                self.save_host_settings();
            }
            if ui.button("Reset rack view").clicked() {
                self.rack.reset_view();
                ui.close();
            }
            if ui.button("Shader editor").clicked() {
                self.shader_open = true;
                ui.close();
            }
            });
        });
        if self.smoke_test.is_some() {
            ctx.data_mut(|data| {
                data.insert_temp(
                    egui::Id::new("smoke-view-menu-popup"),
                    egui::Popup::default_response_id(&view),
                )
            });
        }
    }

    fn toolbar_undo_redo(&mut self, ui: &mut egui::Ui) {
        let undo = icon_button(
            ui,
            Icon::Undo,
            "Undo patch edit · Cmd/Ctrl+Z",
            false,
            self.can_undo,
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-undo-rect"), undo.rect));
        if undo.clicked() {
            self.history(false);
        }
        let redo = icon_button(
            ui,
            Icon::Redo,
            "Redo patch edit · Shift+Cmd/Ctrl+Z",
            false,
            self.can_redo,
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-redo-rect"), redo.rect));
        if redo.clicked() {
            self.history(true);
        }
    }

    fn toolbar_history(&mut self, ui: &mut egui::Ui, history: bool) {
        if history {
            self.toolbar_undo_redo(ui);
        }
        let save = icon_button(ui, Icon::Save, "Save patch · Cmd/Ctrl+S", false, true);
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-save-rect"), save.rect));
        if save.clicked() {
            self.save_current();
        }
    }

    fn toolbar_transport(&mut self, ui: &mut egui::Ui, volume: bool) {
        let playing = self.audio.as_ref().is_some_and(Audio::is_playing);
        let play = icon_button(
            ui,
            if playing { Icon::Pause } else { Icon::Play },
            if playing {
                "Pause audio · Space"
            } else {
                "Play audio · Space"
            },
            playing,
            self.audio.is_some(),
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-play-rect"), play.rect));
        if play.clicked() {
            self.toggle_play();
        }
        if volume {
            self.toolbar_volume(ui);
        }
    }

    fn toolbar_volume(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = 64.0;
            let response = ui
                .add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false))
                .on_hover_text(format!("Master volume · {:.0}%", self.volume * 100.0));
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("toolbar-volume-rect"), response.rect)
            });
            if response.changed() {
                if let Some(audio) = &self.audio {
                    audio.set_volume(self.volume);
                }
            }
        });
    }

    fn toolbar_views(&mut self, ui: &mut egui::Ui) {
        let library = icon_button(
            ui,
            Icon::Library,
            "Module library · Enter",
            self.browser_open,
            true,
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-library-rect"), library.rect));
        if library.clicked() {
            if self.browser_open {
                self.browser_open = false;
            } else {
                self.browse();
            }
        }
        let scope = icon_button(
            ui,
            Icon::Scope,
            "Signal scope",
            self.analysis_tab == Some(AnalysisTab::Scope),
            true,
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-scope-rect"), scope.rect));
        if scope.clicked() {
            self.toggle_analysis(AnalysisTab::Scope);
        }
        let visuals = icon_button(
            ui,
            Icon::Visuals,
            "Visuals",
            self.analysis_tab == Some(AnalysisTab::Visuals),
            true,
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-visuals-rect"), visuals.rect));
        if visuals.clicked() {
            self.toggle_analysis(AnalysisTab::Visuals);
        }
    }

    fn toolbar_zoom(&mut self, ui: &mut egui::Ui) {
        let minus = icon_button(ui, Icon::Minus, "Zoom out", false, true);
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-minus-rect"), minus.rect));
        if minus.clicked() {
            self.rack.zoom_step(false);
        }
        let mut zoom = self.rack.zoom() * 100.0;
        let response = ui
            .add_sized(
                [48.0, 30.0],
                egui::DragValue::new(&mut zoom)
                    .range(25.0..=400.0)
                    .suffix("%")
                    .speed(1.0),
            )
            .on_hover_text("Rack zoom · Cmd/Ctrl −/+ · double-click or Cmd/Ctrl+0 for 100%");
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-zoom-rect"), response.rect));
        if response.double_clicked() {
            self.rack.set_zoom(1.0);
            response.surrender_focus();
            ui.data_mut(|data| data.remove::<String>(response.id));
        } else if response.changed() {
            self.rack.set_zoom(zoom / 100.0);
        }
        let plus = icon_button(ui, Icon::Plus, "Zoom in", false, true);
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-plus-rect"), plus.rect));
        if plus.clicked() {
            self.rack.zoom_step(true);
        }
    }

    fn toolbar_group(ui: &mut egui::Ui, widths: &[f32], contents: impl FnOnce(&mut egui::Ui)) {
        // The parent reserves groups from the right, but each group's controls
        // read left to right. A bounded allocation avoids claiming the title's
        // remaining width when changing the child layout direction.
        let width = widths.iter().sum::<f32>()
            + ui.spacing().item_spacing.x * widths.len().saturating_sub(1) as f32;
        egui::Frame::new()
            .fill(Color32::from_rgb(39, 42, 41))
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(55, 59, 56)))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(4, 3))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 30.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    contents,
                );
            });
    }

    fn toolbar_palette(&mut self, ui: &mut egui::Ui) {
        let enabled = self.library_enabled();
        if self
            .cable_palette_ui
            .toolbar(ui, &mut self.cable_palette, enabled)
        {
            self.save_host_settings();
        }
    }

    fn toolbar_settings_button(&mut self, ui: &mut egui::Ui) {
        let response = icon_button(
            ui,
            Icon::Settings,
            "Audio / MIDI settings",
            self.settings_open,
            true,
        );
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                egui::Id::new("audio-settings-anchor"),
                (response.rect, response.layer_id),
            );
        });
        if response.clicked() {
            self.settings_open = !self.settings_open;
            if !self.settings_open {
                egui::Popup::close_all(ui.ctx());
            }
            if self.settings_open {
                // Unit hosts exercise the real pointer route without touching hardware.
                #[cfg(not(test))]
                self.refresh_devices();
            }
        }
    }

    fn toolbar_project(&self, ui: &mut egui::Ui) {
        let dirty = self.dirty();
        let title = format!("{}{}", self.title, if dirty { " •" } else { "" });
        let state = if dirty {
            "Unsaved"
        } else if self.active_file.is_some() {
            "Saved"
        } else {
            "New patch"
        };
        let width = ui.available_width().min(260.0);
        if width < 65.0 {
            return;
        }
        // Allocate a bounded card in the parent RTL row. A nested horizontal
        // frame otherwise claims the whole row and paints over earlier actions.
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 36.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 8.0, Color32::from_rgb(37, 40, 39));
        painter.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0_f32, Color32::from_rgb(53, 57, 54)),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            3.0,
            if dirty {
                ACCENT
            } else if self.active_file.is_some() {
                Color32::from_rgb(125, 174, 139)
            } else {
                MUTED
            },
        );
        painter.text(
            egui::pos2(rect.left() + 25.0, rect.top() + 5.0),
            egui::Align2::LEFT_TOP,
            state.to_ascii_uppercase(),
            egui::FontId::proportional(8.0),
            if dirty { ACCENT } else { MUTED },
        );
        let mut job = egui::text::LayoutJob::simple(
            title,
            egui::FontId::proportional(12.0),
            TEXT,
            (width - 34.0).max(1.0),
        );
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        painter.galley(
            egui::pos2(rect.left() + 25.0, rect.top() + 17.0),
            galley,
            TEXT,
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("toolbar-project-rect"), rect));
        response.on_hover_text(format!(
            "{}\n{}",
            self.title,
            if dirty {
                "Unsaved changes"
            } else if self.active_file.is_some() {
                "All changes saved"
            } else {
                "New patch · save to keep this project"
            }
        ));
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        let compact = ctx.content_rect().width() < 1140.0;
        let overflow = ctx.content_rect().width() < 850.0;
        let enabled = self.library_enabled();
        // The settings trigger remains available while its dropdown blocks rack edits.
        let chrome_enabled = enabled || self.settings_open;
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(12, 7)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 5.0);
                ui.spacing_mut().button_padding = Vec2::new(7.0, 6.0);
                ui.horizontal(|ui| {
                    ui.set_height(38.0);
                    let (mark, _) =
                        ui.allocate_exact_size(egui::vec2(3.0, 20.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(mark, 1.5, Color32::from_rgb(210, 159, 83));
                    ui.label(RichText::new("OPENPATCH").strong().size(13.0).color(TEXT));
                    ui.add_space(10.0);
                    ui.add_enabled_ui(enabled, |ui| self.toolbar_menus(ui));
                    ui.add_space(12.0);
                    // Reserve each action group before the project gets the remaining width.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(chrome_enabled, |ui| self.toolbar_settings_button(ui));
                        ui.add_enabled_ui(enabled, |ui| {
                            let response = ui.scope(|ui| {
                                self.mcp_session.toolbar(
                                    ui,
                                    &self.shared,
                                    Path::new(&self.workspace),
                                )
                            });
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    egui::Id::new("toolbar-mcp-rect"),
                                    response.response.rect,
                                )
                            });
                            ui.add_space(8.0);
                            if !compact {
                                self.toolbar_palette(ui);
                                ui.add_space(8.0);
                                Self::toolbar_group(ui, &[30.0, 48.0, 30.0], |ui| {
                                    self.toolbar_zoom(ui)
                                });
                                ui.add_space(8.0);
                                Self::toolbar_group(ui, &[30.0, 30.0, 30.0], |ui| {
                                    self.toolbar_views(ui)
                                });
                                ui.add_space(8.0);
                            }
                            Self::toolbar_group(
                                ui,
                                if compact { &[30.0] } else { &[30.0, 64.0] },
                                |ui| self.toolbar_transport(ui, !compact),
                            );
                            ui.add_space(8.0);
                            Self::toolbar_group(
                                ui,
                                if overflow {
                                    &[30.0]
                                } else {
                                    &[30.0, 30.0, 30.0]
                                },
                                |ui| self.toolbar_history(ui, !overflow),
                            );
                        });
                        ui.add_space(6.0);
                        self.toolbar_project(ui);
                    });
                });
                if compact {
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("RACK").strong().size(9.0).color(MUTED));
                            Self::toolbar_group(ui, &[30.0, 30.0, 30.0], |ui| {
                                self.toolbar_views(ui)
                            });
                            ui.add_space(14.0);
                            Self::toolbar_group(ui, &[30.0, 48.0, 30.0], |ui| {
                                self.toolbar_zoom(ui)
                            });
                            ui.add_space(14.0);
                            if overflow {
                                egui::containers::menu::MenuButton::new("More")
                                    .config(
                                        egui::containers::menu::MenuConfig::new().close_behavior(
                                            egui::PopupCloseBehavior::CloseOnClickOutside,
                                        ),
                                    )
                                    .ui(ui, |ui| {
                                        ui.horizontal(|ui| self.toolbar_undo_redo(ui));
                                        ui.separator();
                                        ui.label("Master volume");
                                        self.toolbar_volume(ui);
                                        ui.separator();
                                        self.toolbar_palette(ui);
                                    });
                            } else {
                                self.toolbar_palette(ui);
                                ui.add_space(14.0);
                                self.toolbar_volume(ui);
                            }
                        });
                    });
                }
            });
    }
}
