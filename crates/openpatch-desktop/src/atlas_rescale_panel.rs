// Configured affine/bound path. Crossing lamps are exclusively engine snapshots.
fn atlas_rescale_control(name: &str) -> (Pos2, f32) {
    (
        pos2(
            if matches!(name, "gain" | "min") {
                60.0
            } else {
                180.0
            },
            if matches!(name, "gain" | "offset") {
                127.0
            } else {
                243.0
            },
        ),
        25.0,
    )
}
fn atlas_rescale_value_center(name: &str) -> Pos2 {
    let (p, _) = atlas_rescale_control(name);
    pos2(p.x, if p.y < 200.0 { 166.0 } else { 282.0 })
}
fn paint_atlas_rescale_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let (p, _) = atlas_rescale_control(name);
    painter.text(
        rect.min + vec2(p.x, if p.y < 200.0 { 91.0 } else { 207.0 }) * zoom,
        Align2::CENTER_CENTER,
        match name {
            "gain" => "GAIN · SIGNED",
            "offset" => "OFFSET · V",
            "min" => "MINIMUM · V",
            _ => "MAXIMUM · V",
        },
        crate::panel_skin::print_font(painter.ctx(), 9.0 * zoom),
        INK,
    );
}
fn atlas_rescale_setting_rect(name: &str, value: f32) -> Rect {
    if name == "multiplier" {
        let index = [1.0, 10.0, 100.0, 1000.0]
            .iter()
            .position(|v| *v == value)
            .unwrap_or(0);
        Rect::from_min_size(pos2(65.0 + index as f32 * 39.0, 65.0), vec2(36.0, 18.0))
    } else {
        Rect::from_center_size(
            pos2(if name == "reflect_min" { 60.0 } else { 180.0 }, 318.0),
            vec2(78.0, 16.0),
        )
    }
}
fn atlas_rescale_bound_guide(node: &Node) -> &'static str {
    let p = |name, fallback| node.params.get(name).copied().unwrap_or(fallback);
    if p("max", 10.0) <= p("min", -10.0) {
        "MAX ≤ MIN: OUTPUT = MIN"
    } else {
        match (p("reflect_min", 0.0) != 0.0, p("reflect_max", 0.0) != 0.0) {
            (false, false) => "AFFINE → CLIP TO MIN…MAX",
            (true, true) => "AFFINE → REPEATED REFLECTION",
            (true, false) => "REFLECT MIN ONCE → CLIP MAX",
            (false, true) => "REFLECT MAX ONCE → CLIP MIN",
        }
    }
}
impl Rack {
    fn atlas_rescale_panel(
        &mut self,
        ui: &mut egui::Ui,
        node: &Node,
        rect: Rect,
        revision: u64,
        interactive: bool,
    ) -> Option<RackAction> {
        let painter = ui.painter().clone();
        let zoom = self.zoom;
        let point = |x, y| rect.min + vec2(x, y) * zoom;
        let screen = |r: Rect| Rect::from_min_max(point(r.min.x, r.min.y), point(r.max.x, r.max.y));
        let text = |x, y, label: &str, size, color| {
            painter.text(
                point(x, y),
                Align2::CENTER_CENTER,
                label,
                crate::panel_skin::print_font(ui.ctx(), size * zoom),
                color,
            );
        };
        let p = |name, fallback| node.params.get(name).copied().unwrap_or(fallback);
        let multiplier = p("multiplier", 1.0);
        let allowed = interactive
            && ui.is_enabled()
            && self.shortcuts_enabled
            && self.editing.is_none()
            && self.annotation_edit.is_none()
            && self.blank_resize.is_none()
            && ui.input(|i| i.focused)
            && !egui::Popup::is_any_open(ui.ctx())
            && !ui.input(|i| i.key_pressed(egui::Key::Escape));
        text(36.0, 74.0, "GAIN ×", 8.0, MUTED);
        let mut action = None;
        for name in ["multiplier", "reflect_min", "reflect_max"] {
            let current = p(name, if name == "multiplier" { 1.0 } else { 0.0 });
            let values: &[f32] = if name == "multiplier" {
                &[1.0, 10.0, 100.0, 1000.0]
            } else {
                &[0.0]
            };
            let mut ids = Vec::new();
            let mut anchor = Rect::NOTHING;
            for &value in values {
                let bounds = screen(atlas_rescale_setting_rect(name, value));
                anchor = anchor.union(bounds);
                let response = ui.interact(
                    bounds,
                    ui.id()
                        .with((&node.id, "rescale-setting", name, value.to_bits())),
                    Sense::click(),
                );
                ids.push(response.id);
                let label = if name == "multiplier" {
                    format!("{value:.0}×")
                } else if current == 0.0 {
                    "CLIP".into()
                } else {
                    "REFLECT".into()
                };
                let selected = if name == "multiplier" {
                    current == value
                } else {
                    current != 0.0
                };
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        allowed,
                        selected,
                        format!("{} {name}: {label}", node.id),
                    )
                });
                painter.rect_filled(
                    bounds,
                    2.0 * zoom,
                    if selected {
                        Color32::from_rgb(215, 177, 108)
                    } else {
                        Color32::from_rgb(231, 221, 193)
                    },
                );
                painter.rect_stroke(
                    bounds,
                    2.0 * zoom,
                    Stroke::new(
                        if response.has_focus() {
                            1.4 * zoom
                        } else {
                            0.6 * zoom
                        },
                        Color32::from_rgb(145, 126, 88),
                    ),
                    egui::StrokeKind::Inside,
                );
                painter.text(
                    bounds.center(),
                    Align2::CENTER_CENTER,
                    &label,
                    crate::panel_skin::print_font(ui.ctx(), 8.0 * zoom),
                    INK,
                );
                let keyboard = allowed
                    && response.has_focus()
                    && ui.input_mut(|i| {
                        let m = i.modifiers;
                        i.consume_key(m, egui::Key::Space) || i.consume_key(m, egui::Key::Enter)
                    });
                if allowed && (response.clicked() || keyboard) {
                    response.request_focus();
                    self.select_primary(node.id.clone());
                    let next = if name == "multiplier" {
                        value
                    } else if current == 0.0 {
                        1.0
                    } else {
                        0.0
                    };
                    if next != current {
                        action = Some(RackAction::Command(
                            "set_parameter",
                            json!({"id":node.id,"parameter":name,"value":next,"expected_revision":revision}),
                        ));
                    }
                }
                response.on_hover_text(if name == "multiplier" {"Saved gain multiplier. The gain readout and exact-value editor show raw signed gain × this multiplier; the knob retains its −1…+1 travel."} else {"Clip this bound, or reflect at it. One reflected bound mirrors once and clips at the other. Both reflected bounds fold repeatedly using positive modulo. If max ≤ min, output is always min."});
            }
            if interactive {
                if let Some(cue) = self
                    .construction_animation
                    .parameter(&node.id, &node.kind, name)
                {
                    let label = |v: f32| {
                        if name == "multiplier" {
                            format!("{v:.0}×")
                        } else if v == 0.0 {
                            "Clip".into()
                        } else {
                            "Reflect".into()
                        }
                    };
                    self.register_construction_anchor(
                        ui,
                        node,
                        name,
                        crate::construction_motion::Anchor {
                            rect: anchor,
                            old_text: label(cue.old),
                            new_text: label(cue.new),
                            marker: None,
                        },
                        &ids,
                    );
                }
            }
        }
        let guide = screen(Rect::from_min_max(pos2(16.0, 176.0), pos2(224.0, 198.0)));
        painter.rect_filled(guide, 2.0 * zoom, Color32::from_rgb(220, 216, 196));
        text(
            120.0,
            182.0,
            &format!(
                "IN × {:+.3} + {:+.2} V",
                p("gain", 0.0) * multiplier,
                p("offset", 0.0)
            ),
            8.0,
            INK,
        );
        text(120.0, 193.0, atlas_rescale_bound_guide(node), 7.0, MUTED);
        ui.interact(guide,ui.id().with((&node.id,"rescale-guide")),Sense::hover()).on_hover_text("Configured transfer, not a measured waveform. Signed gain multiplies input polarity; offset is added afterward. Limiting/reflection follows the affine stage. Empty input processes one zero-valued lane.");
        let states = self.module_lights.get(&node.id);
        for (index, x, name) in [(0, 180.0, "max"), (1, 60.0, "min")] {
            let brightness = states
                .filter(|s| index < usize::from(s.count))
                .map_or([0.0; 2], |s| s.brightness[index]);
            let center = point(x, 299.0);
            paint_signal_light(&painter, center, brightness, zoom, &self.lighting);
            ui.interact(Rect::from_center_size(center,vec2(54.0,12.0)*zoom),ui.id().with((&node.id,"rescale-crossing",index)),Sense::hover()).on_hover_text(format!("Pre-bound {name} crossing (inclusive): yellow mono, blue poly. Engine snapshot before clipping/reflection; bypass freezes lights."));
        }
        text(
            120.0,
            334.0,
            if node.bypassed {
                "BYPASSED · LAMPS FROZEN"
            } else if states.is_none() {
                "CROSSINGS · NO ENGINE SNAPSHOT"
            } else if self.performance_available {
                "CROSSINGS · YELLOW MONO / BLUE POLY"
            } else {
                "CROSSINGS · OFFLINE SNAPSHOT"
            },
            6.5,
            MUTED,
        );
        action
    }
}
