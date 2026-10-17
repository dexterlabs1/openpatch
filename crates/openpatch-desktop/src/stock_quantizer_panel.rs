// Original shared Stock Quantizer panel. Enabled notes are configuration;
// played highlights are a separate measured union, never inferred from a scale.
const STOCK_QUANTIZER_NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
struct ScalePaint {
    id: String,
    gesture: u64,
    revision: u64,
    mask: u16,
    paint_on: bool,
    visited: u16,
    pending: Option<u16>,
    history_group: String,
}
fn stock_quantizer_offset_value(raw: f32) -> String {
    format!("{:+.2} st", raw * 12.0)
}
fn stock_quantizer_mask(node: &Node) -> u16 {
    node.params
        .get("scale_mask")
        .copied()
        .unwrap_or(4095.0)
        .clamp(0.0, 4095.0) as u16
}
fn stock_quantizer_key_rect(note: usize) -> Rect {
    let width = 260.0 / 7.0;
    if let Some(column) = [0, 2, 4, 5, 7, 9, 11].iter().position(|n| *n == note) {
        Rect::from_min_size(
            pos2(20.0 + column as f32 * width, 100.0),
            vec2(width - 1.0, 99.0),
        )
    } else {
        let boundary = match note {
            1 => 1,
            3 => 2,
            6 => 4,
            8 => 5,
            _ => 6,
        };
        Rect::from_min_size(
            pos2(20.0 + boundary as f32 * width - width * 0.3, 100.0),
            vec2(width * 0.6, 60.0),
        )
    }
}
fn stock_quantizer_hit(point: Pos2) -> Option<usize> {
    [1, 3, 6, 8, 10, 0, 2, 4, 5, 7, 9, 11]
        .into_iter()
        .find(|note| stock_quantizer_key_rect(*note).contains(point))
}
fn stock_quantizer_scale_label(mask: u16) -> String {
    if mask == 0 {
        "None (chromatic fallback)".into()
    } else if mask == 4095 {
        "Chromatic".into()
    } else {
        STOCK_QUANTIZER_NOTES
            .iter()
            .enumerate()
            .filter(|(note, _)| mask & (1 << note) != 0)
            .map(|(_, name)| *name)
            .collect::<Vec<_>>()
            .join(" ")
    }
}
impl Rack {
    pub(crate) fn complete_scale_paint(&mut self, gesture: u64, result: Result<u64, String>) {
        let Some(paint) = self
            .scale_paint
            .as_mut()
            .filter(|paint| paint.gesture == gesture)
        else {
            return;
        };
        if let Ok(revision) = result {
            paint.revision = revision;
            if let Some(mask) = paint.pending.take() {
                paint.mask = mask;
            }
        } else {
            self.scale_paint = None;
        }
    }
    fn validate_scale_paint(
        &mut self,
        ui: &egui::Ui,
        patch: &Patch,
        revision: u64,
        interactive: bool,
    ) {
        if self.scale_paint.as_ref().is_some_and(|paint| {
            !interactive
                || !ui.input(|i| i.focused && i.pointer.primary_down())
                || self.editing.is_some()
                || self.annotation_edit.is_some()
                || ui.input(|i| i.key_pressed(egui::Key::Escape))
                || !patch.nodes.iter().any(|node| {
                    node.id == paint.id
                        && node.kind == "stock_quantizer"
                        && revision == paint.revision
                        && (paint.pending.is_some() || stock_quantizer_mask(node) == paint.mask)
                })
        }) {
            self.scale_paint = None;
        }
    }
    fn stock_quantizer_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let zoom = self.zoom;
        let painter = ui.painter().clone();
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let screen = |r: Rect| Rect::from_min_max(point(r.min.x, r.min.y), point(r.max.x, r.max.y));
        let mask = self
            .stock_quantizer_states
            .get(&node.id)
            .copied()
            .unwrap_or_else(|| stock_quantizer_mask(node));
        let played = self
            .module_lights
            .get(&node.id)
            .and_then(|packet| packet.playing_notes)
            .filter(|mask| *mask <= 4095);
        painter.text(
            point(20.0, 83.0),
            Align2::LEFT_CENTER,
            "ENABLED NOTES",
            crate::panel_skin::print_font(ui.ctx(), 11.0 * zoom),
            INK,
        );
        painter.text(
            point(280.0, 83.0),
            Align2::RIGHT_CENTER,
            if played.is_some() {
                "PLAYED NOTES"
            } else {
                "PLAYED —"
            },
            crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom),
            MUTED,
        );
        // Keep the printed port row clear of both socket surrounds, including
        // raster-rounded glyph heights in small shared browser previews.
        for (x, output) in [(85.0, false), (215.0, true)] {
            paint_enlarged_port_label(
                &painter,
                point(if output { x + 36.0 } else { x - 34.0 }, 376.0),
                "V/OCT",
                output,
                zoom,
            );
        }
        let allowed = interactive
            && ui.is_enabled()
            && self.shortcuts_enabled
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && ui.input(|i| i.focused)
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.input(|i| i.key_pressed(egui::Key::Escape));
        let mut ids = Vec::with_capacity(12);
        let mut origin = None;
        for note in [0, 2, 4, 5, 7, 9, 11, 1, 3, 6, 8, 10] {
            let black = matches!(note, 1 | 3 | 6 | 8 | 10);
            let bounds = screen(stock_quantizer_key_rect(note));
            let enabled = mask & (1 << note) != 0;
            let response = ui.interact(
                bounds,
                ui.id().with((&node.id, "stock-quantizer-key", note)),
                Sense::click_and_drag(),
            );
            ids.push(response.id);
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Checkbox,
                    allowed,
                    enabled,
                    format!("{} scale note {}", node.id, STOCK_QUANTIZER_NOTES[note]),
                )
            });
            painter.rect_filled(
                bounds,
                1.0 * zoom,
                match (black, enabled) {
                    (false, true) => Color32::from_rgb(225, 203, 152),
                    (false, false) => Color32::from_rgb(237, 233, 220),
                    (true, true) => Color32::from_rgb(137, 103, 54),
                    (true, false) => Color32::from_rgb(37, 43, 43),
                },
            );
            painter.rect_stroke(
                bounds,
                1.0 * zoom,
                Stroke::new(0.8 * zoom, Color32::from_rgb(99, 108, 101)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                bounds.center_bottom() - vec2(0.0, 10.0) * zoom,
                Align2::CENTER_CENTER,
                STOCK_QUANTIZER_NOTES[note],
                crate::panel_skin::print_font(
                    ui.ctx(),
                    if black { 8.5 * zoom } else { 11.0 * zoom },
                ),
                if black {
                    Color32::from_rgb(239, 239, 222)
                } else {
                    INK
                },
            );
            if played.is_some_and(|playing| playing & (1 << note) != 0) {
                let emitter = Rect::from_center_size(
                    bounds.center_bottom() - vec2(0.0, 24.0) * zoom,
                    vec2(if black { 12.0 } else { 22.0 }, 3.0) * zoom,
                );
                let start = crate::lighting::next(&painter);
                painter.rect_filled(emitter, 0.8 * zoom, Color32::from_rgb(75, 232, 218));
                self.lighting.preserve(&painter, start);
            }
            if allowed
                && ((ui.input(|i| i.pointer.button_pressed(PointerButton::Primary))
                    && (response.is_pointer_button_down_on()
                        || response.clicked_by(PointerButton::Primary)))
                    || (response.has_focus()
                        && ui.input_mut(|i| {
                            let modifiers = i.modifiers;
                            i.consume_key(modifiers, egui::Key::Space)
                                || i.consume_key(modifiers, egui::Key::Enter)
                        })))
            {
                response.request_focus();
                origin = Some(note);
            }
            response.on_hover_text(format!("{}: {}. Click to toggle; drag paints the origin's new value. Cyan marks actual output notes, even when a note is disabled.",STOCK_QUANTIZER_NOTES[note],if enabled{"enabled"}else{"disabled"}));
        }
        let keyboard = screen(Rect::from_min_max(pos2(20.0, 100.0), pos2(280.0, 199.0)));
        if interactive {
            if let Some(cue) =
                self.construction_animation
                    .parameter(&node.id, &node.kind, "scale_mask")
            {
                self.register_construction_anchor(
                    ui,
                    node,
                    "scale_mask",
                    crate::construction_motion::Anchor {
                        rect: keyboard,
                        old_text: stock_quantizer_scale_label(cue.old as u16),
                        new_text: stock_quantizer_scale_label(cue.new as u16),
                        marker: None,
                    },
                    &ids,
                );
            }
        }
        let mut action = None;
        if let Some(note) = origin {
            self.gesture = self.gesture.wrapping_add(1);
            self.scale_paint = Some(ScalePaint {
                id: node.id.clone(),
                gesture: self.gesture,
                revision,
                mask,
                paint_on: mask & (1 << note) == 0,
                visited: 0,
                pending: None,
                history_group: format!("stock-quantizer-paint-{}", self.gesture),
            });
        }
        let pointer = ui.input(|i| i.pointer.interact_pos());
        let target = origin.or_else(|| {
            pointer
                .filter(|p| {
                    ui.clip_rect().contains(*p) && ui.ctx().layer_id_at(*p) == Some(ui.layer_id())
                })
                .and_then(|p| stock_quantizer_hit(((p - rect.min) / zoom).to_pos2()))
        });
        if allowed {
            if let Some(paint) = self
                .scale_paint
                .as_mut()
                .filter(|paint| paint.id == node.id && paint.pending.is_none())
            {
                if let Some(note) =
                    target.filter(|_| ui.input(|i| i.pointer.primary_down()) || origin.is_some())
                {
                    let bit = 1_u16 << note;
                    if paint.visited & bit == 0 {
                        paint.visited |= bit;
                        let next = if paint.paint_on {
                            paint.mask | bit
                        } else {
                            paint.mask & !bit
                        };
                        if next != paint.mask {
                            paint.pending = Some(next);
                            action = Some(RackAction::ScalePaint {
                                gesture: paint.gesture,
                                arguments: json!({"id":node.id,"parameter":"scale_mask","value":next,"expected_revision":paint.revision,"history_group":paint.history_group}),
                            });
                        }
                    }
                }
            }
        }
        painter.text(
            point(150.0, 212.0),
            Align2::CENTER_CENTER,
            "AMBER: ENABLED  ·  CYAN: PLAYED",
            crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
            MUTED,
        );
        for (x, label, semitones) in [(20.0, "−1 semitone", -1), (205.0, "+1 semitone", 1)] {
            if panel_button(
                ui,
                &painter,
                Rect::from_min_size(point(x, 224.0), vec2(75.0, 24.0) * zoom),
                ui.id().with((&node.id, "stock-quantizer-shift", semitones)),
                label,
                false,
                true,
                zoom,
            ) && interactive
            {
                self.scale_paint = None;
                self.construction_animation
                    .interrupt_parameter(&node.id, "scale_mask");
                action = Some(RackAction::Command(
                    "rotate_scale",
                    json!({"id":node.id,"semitones":semitones,"expected_revision":revision}),
                ));
            }
        }
        let menu_rect = Rect::from_min_size(point(106.0, 224.0), vec2(88.0, 24.0) * zoom);
        let mut preset = None;
        let response = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(menu_rect)
                    .id_salt((&node.id, "stock-quantizer-presets")),
                |ui| {
                    *ui.visuals_mut() = egui::Visuals::light();
                    crate::panel_skin::control_style(ui, zoom);
                    ui.style_mut().override_font_id =
                        Some(crate::panel_skin::print_font(ui.ctx(), 10.0 * zoom));
                    let spacing = ui.spacing_mut();
                    spacing.button_padding = vec2(4.0, 3.0) * zoom;
                    spacing.icon_width = 12.0 * zoom;
                    spacing.icon_width_inner = 7.0 * zoom;
                    spacing.icon_spacing = 3.0 * zoom;
                    spacing.interact_size = vec2(30.0, 18.0) * zoom;
                    spacing.item_spacing = vec2(4.0, 3.0) * zoom;
                    egui::ComboBox::from_id_salt("scale")
                        .width(menu_rect.width())
                        .selected_text("Presets")
                        .show_ui(ui, |ui| {
                            for (label, choice) in [
                                ("Chromatic", 4095_u16),
                                ("Major", 2741),
                                ("Minor", 1453),
                                ("Pentatonic", 661),
                                ("None", 0),
                            ] {
                                if ui.selectable_label(mask == choice, label).clicked() {
                                    preset = Some(choice);
                                    ui.close();
                                }
                            }
                        })
                        .response
                },
            )
            .inner;
        let opening = response.id.with("opening-revision");
        if interactive && response.clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(opening, revision));
            self.scale_paint = None;
            self.construction_animation
                .interrupt_parameter(&node.id, "scale_mask");
        }
        if interactive {
            if let Some(preset) = preset.filter(|choice| *choice != mask) {
                let revision = ui
                    .ctx()
                    .data(|d| d.get_temp::<u64>(opening))
                    .unwrap_or(revision);
                action = Some(RackAction::Command(
                    "set_parameter",
                    json!({"id":node.id,"parameter":"scale_mask","value":preset,"expected_revision":revision}),
                ));
            }
        }
        action
    }
}
