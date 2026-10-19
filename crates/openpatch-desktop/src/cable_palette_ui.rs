//! Native next-cable chooser and explicitly applied palette drafts.
use crate::cable_palette::{Palette, Preset, MAX_COLORS, MAX_NAME_CHARS, MAX_PRESETS};
use eframe::egui::{self, vec2, Color32, Context, Id, Rect, Response, Sense, Stroke, Ui};

const AMBER: Color32 = Color32::from_rgb(226, 175, 88);
const MUTED: Color32 = Color32::from_rgb(157, 158, 151);

/// Host cable material choice. Native buttons retain keyboard navigation and
/// accessible names; vector swatches communicate the jacket before selection.
pub(crate) fn cable_finish_picker(
    ui: &mut Ui,
    finish: &mut crate::cable_presentation::CableFinish,
) -> bool {
    use crate::cable_presentation::CableFinish;
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for candidate in CableFinish::ALL {
            let response = ui.add_sized(
                vec2(104.0, 66.0),
                egui::Button::new(format!("\n\n{}", candidate.label()))
                    .selected(*finish == candidate),
            );
            let painter = ui
                .painter()
                .with_clip_rect(response.rect.intersect(ui.clip_rect()));
            let origin = response.rect.min + vec2(13.0, 14.0);
            let point = |t: f32| {
                let inverse = 1.0 - t;
                origin
                    + vec2(0.0, 0.0) * inverse.powi(3)
                    + vec2(19.0, 25.0) * (3.0 * inverse.powi(2) * t)
                    + vec2(58.0, 25.0) * (3.0 * inverse * t.powi(2))
                    + vec2(78.0, 0.0) * t.powi(3)
            };
            let points: Vec<_> = (0..=24).map(|index| point(index as f32 / 24.0)).collect();
            let stroke = |width: f32, color| {
                painter.add(egui::Shape::line(points.clone(), Stroke::new(width, color)));
            };
            stroke(7.0, Color32::from_black_alpha(110));
            match candidate {
                CableFinish::Classic => {
                    stroke(5.2, Color32::from_rgb(169, 112, 51));
                    stroke(2.2, AMBER);
                }
                CableFinish::Woven => {
                    stroke(5.2, Color32::from_rgb(164, 124, 73));
                    for index in 2..23 {
                        let t = index as f32 / 24.0;
                        let tangent = (point(t + 0.01) - point(t - 0.01)).normalized();
                        let diagonal = (vec2(-tangent.y, tangent.x) + tangent * 0.45) * 2.0;
                        painter.line_segment(
                            [point(t) - diagonal, point(t) + diagonal],
                            Stroke::new(
                                0.8_f32,
                                if index % 2 == 0 {
                                    Color32::from_rgb(216, 178, 117)
                                } else {
                                    Color32::from_rgb(112, 82, 49)
                                },
                            ),
                        );
                    }
                }
                CableFinish::Tracer => {
                    stroke(5.2, Color32::from_rgb(114, 87, 56));
                    stroke(1.4, Color32::from_rgb(239, 195, 118));
                }
                CableFinish::Translucent => {
                    stroke(5.8, Color32::from_rgba_unmultiplied(226, 175, 88, 105));
                    stroke(2.0, Color32::from_rgb(163, 127, 75));
                    stroke(0.7, Color32::from_rgb(223, 201, 160));
                }
            }
            for endpoint in [point(0.0), point(1.0)] {
                painter.circle_filled(endpoint, 3.5, Color32::from_rgb(39, 41, 38));
                painter.circle_stroke(
                    endpoint,
                    2.7,
                    Stroke::new(0.7_f32, Color32::from_rgb(137, 137, 125)),
                );
            }
            if response.clicked() && *finish != candidate {
                *finish = candidate;
                changed = true;
            }
        }
    });
    changed
}

#[derive(Clone)]
struct Draft {
    presets: Vec<Preset>,
    active: usize,
    next: usize,
    generation: u64,
}

impl Draft {
    fn from_palette(palette: &Palette) -> Self {
        Self {
            presets: palette.presets().to_vec(),
            active: palette.active_index(),
            next: palette.next_index(),
            generation: palette.selection().generation,
        }
    }

    fn copy_preset(&mut self) {
        if self.presets.len() >= MAX_PRESETS {
            return;
        }
        let mut copy = self.presets[self.active].clone();
        let stem: String = copy.name.trim().chars().take(MAX_NAME_CHARS - 5).collect();
        let mut suffix = 2;
        loop {
            copy.name = format!("{stem} ({suffix})");
            if !self
                .presets
                .iter()
                .any(|preset| preset.name.trim().to_lowercase() == copy.name.trim().to_lowercase())
            {
                break;
            }
            suffix += 1;
        }
        self.presets.push(copy);
        self.active = self.presets.len() - 1;
    }

    fn delete_preset(&mut self) {
        if self.presets.len() > 1 {
            self.presets.remove(self.active);
            self.active = self.active.min(self.presets.len() - 1);
            self.next = self.next.min(self.presets[self.active].colors.len() - 1);
        }
    }

    fn move_color(&mut self, forward: bool) {
        let colors = &mut self.presets[self.active].colors;
        let target = if forward {
            (self.next + 1).min(colors.len() - 1)
        } else {
            self.next.saturating_sub(1)
        };
        colors.swap(self.next, target);
        self.next = target;
    }
}

#[derive(Default)]
pub(crate) struct Panel {
    open: bool,
    draft: Option<Draft>,
    error: Option<String>,
    focus_ids: Vec<Id>,
    window_rect: Option<Rect>,
    window_layer: Option<egui::LayerId>,
    keyboard_active: bool,
}

impl Panel {
    pub(crate) fn open(&mut self) {
        if !self.open {
            self.draft = None;
            self.error = None;
        }
        self.open = true;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn keyboard_active(&self) -> bool {
        self.keyboard_active
    }

    /// Returns true only when a committed setting changes.
    pub(crate) fn toolbar(&mut self, ui: &mut Ui, palette: &mut Palette, enabled: bool) -> bool {
        let mut changed = false;
        let color = palette.selection().color;
        let button = ui
            .add_enabled(enabled, |ui: &mut Ui| {
                let (rect, response) = ui.allocate_exact_size(vec2(86.0, 30.0), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        format!("Next cable color {}. Open cable palette", hex(color)),
                    )
                });
                let visuals = ui.style().interact(&response);
                ui.painter().rect_filled(rect, 5.0, visuals.bg_fill);
                ui.painter().rect_stroke(
                    rect,
                    5.0,
                    Stroke::new(
                        1.0_f32,
                        if response.has_focus() {
                            AMBER
                        } else {
                            visuals.bg_stroke.color
                        },
                    ),
                    egui::StrokeKind::Inside,
                );
                let ink = if ui.is_enabled() {
                    rgb(color)
                } else {
                    MUTED.gamma_multiply(0.5)
                };
                let start = rect.left_center() + vec2(9.0, 0.0);
                ui.painter()
                    .line_segment([start, start + vec2(19.0, 0.0)], Stroke::new(5.0_f32, ink));
                ui.painter().circle_filled(start, 3.0, ink);
                ui.painter()
                    .circle_filled(start + vec2(19.0, 0.0), 3.0, ink);
                ui.painter().text(
                    rect.center() + vec2(10.0, 0.0),
                    egui::Align2::CENTER_CENTER,
                    "Cable",
                    egui::FontId::proportional(11.0),
                    visuals.text_color(),
                );
                let tip = rect.right_center() - vec2(8.0, 0.0);
                ui.painter().add(egui::Shape::line(
                    vec![
                        tip + vec2(-3.0, -1.5),
                        tip + vec2(0.0, 1.5),
                        tip + vec2(3.0, -1.5),
                    ],
                    Stroke::new(1.0_f32, visuals.text_color()),
                ));
                response
            })
            .on_hover_text(format!(
                "Next cable: {}\nChoose a color or edit palettes",
                hex(color)
            ));
        egui::Popup::from_toggle_button_response(&button)
            .kind(egui::PopupKind::Menu)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(264.0)
            .show(|ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 8.0);
                    ui.horizontal(|ui| {
                        ui.strong("Next cable");
                    });
                    let mut active = palette.active_index();
                    egui::containers::menu::SubMenuButton::from_button(
                        egui::Button::new(format!("Palette: {}", palette.presets()[active].name))
                            .right_text(">"),
                    )
                    .ui(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(210.0)
                            .show(ui, |ui| {
                                for (index, preset) in palette.presets().iter().enumerate() {
                                    if ui
                                        .selectable_value(&mut active, index, &preset.name)
                                        .clicked()
                                    {
                                        ui.close();
                                    }
                                }
                            });
                    });
                    changed |= palette.select_preset(active);
                    ui.horizontal_wrapped(|ui| {
                        for (index, color) in palette.presets()[palette.active_index()]
                            .colors
                            .clone()
                            .into_iter()
                            .enumerate()
                        {
                            if swatch(ui, color, index == palette.next_index(), index).clicked() {
                                changed |= palette.select_color(index);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui
                            .button("Previous")
                            .on_hover_text("Use the previous cable color")
                            .clicked()
                        {
                            changed |= palette.cycle(false);
                        }
                        if ui
                            .button("Next")
                            .on_hover_text("Use the next cable color")
                            .clicked()
                        {
                            changed |= palette.cycle(true);
                        }
                    });
                    let mut automatic = palette.auto_advance();
                    if ui
                        .checkbox(&mut automatic, "Advance after each cable")
                        .on_hover_text("Automatically choose the next color after creating a cable")
                        .changed()
                    {
                        changed |= palette.set_auto_advance(automatic);
                    }
                    ui.separator();
                    if ui.button("Edit palettes…").clicked() {
                        self.open();
                        ui.close();
                    }
                });
            });
        changed
    }

    /// A draft remains local until Apply. Outside clicks relinquish keyboard focus.
    pub(crate) fn show(&mut self, ctx: &Context, palette: &mut Palette, enabled: bool) -> bool {
        if !self.open {
            self.keyboard_active = false;
            return false;
        }
        if self.draft.is_none() {
            self.draft = Some(Draft::from_palette(palette));
        }
        let focused = ctx.memory(|memory| memory.focused());
        // egui clears the focused widget before delivering Escape. Remember
        // ownership from the last frame so Escape can dismiss this editor.
        let owns_focus = focused.is_some_and(|id| self.focus_ids.contains(&id))
            || (focused.is_none() && self.keyboard_active);
        let (pressed, released, pointer) = ctx.input(|input| {
            (
                input.pointer.any_pressed(),
                input.pointer.any_released(),
                input.pointer.interact_pos(),
            )
        });
        let outside_press = pressed
            && pointer.is_some_and(|pos| {
                !self.window_rect.is_some_and(|rect| rect.contains(pos))
                    || ctx.layer_id_at(pos) != self.window_layer
            });
        if owns_focus && (outside_press || !enabled || !ctx.input(|input| input.focused)) {
            if let Some(id) = focused {
                ctx.memory_mut(|memory| memory.surrender_focus(id));
            }
            self.keyboard_active = false;
        }
        if enabled
            && owns_focus
            && !outside_press
            && ctx.input(|input| input.focused)
            && !egui::Popup::is_any_open(ctx)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.close(ctx);
            return false;
        }
        let viewport = ctx.content_rect().shrink(8.0);
        let mut open = true;
        let mut cancel = false;
        let mut apply = false;
        let mut reload = false;
        let mut ids = Vec::new();
        let draft = self.draft.as_mut().unwrap();
        let stale = draft.generation != palette.selection().generation;
        let shown = egui::Window::new("Cable palettes")
            .id(Id::new("cable-palette-editor"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(viewport.min + vec2(90.0, 70.0))
            .default_width(420.0)
            .min_width(280.0_f32.min(viewport.width()))
            .max_width(560.0_f32.min(viewport.width()))
            .max_height(viewport.height())
            .constrain_to(viewport)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = vec2(7.0, 9.0);
                ui.add_enabled_ui(enabled, |ui| {
                    let background = ui.interact(
                        ui.max_rect(),
                        ui.id().with("focus"),
                        Sense::focusable_noninteractive(),
                    );
                    ids.push(background.id);
                    ui.label(egui::RichText::new("Make every connection yours.").color(MUTED));
                    egui::ScrollArea::vertical()
                        .id_salt("cable-palette-draft")
                        .max_height((viewport.height() - 190.0).clamp(80.0, 450.0))
                        .show(ui, |ui| {
                            ui.add_enabled_ui(!stale, |ui| {
                                editor_contents(ui, draft, &mut ids);
                            });
                        });
                    if stale {
                        ui.colored_label(
                            AMBER,
                            "The cable palette changed while this draft was open.",
                        );
                        let response = ui.button("Reload current palette");
                        reload = response.clicked();
                        ids.push(response.id);
                    }
                    if let Some(error) = &self.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        let response = ui.add_enabled(
                            !stale,
                            egui::Button::new(egui::RichText::new("Apply").color(AMBER)),
                        );
                        apply = response.clicked();
                        ids.push(response.id);
                        let response = ui.button("Cancel");
                        cancel = response.clicked();
                        ids.push(response.id);
                        ui.small("Changes apply when saved.");
                    });
                });
            });
        self.window_rect = shown.as_ref().map(|shown| shown.response.rect);
        self.window_layer = shown.as_ref().map(|shown| shown.response.layer_id);
        self.focus_ids = ids;
        // Clicking blank editor space owns focus; simply hovering never does.
        if enabled {
            if let Some(shown) = &shown {
                let inside_release = released
                    && pointer.is_some_and(|pos| {
                        shown.response.rect.contains(pos)
                            && ctx.layer_id_at(pos) == Some(shown.response.layer_id)
                    });
                if inside_release
                    && !ctx.memory(|memory| {
                        memory
                            .focused()
                            .is_some_and(|id| self.focus_ids.contains(&id))
                    })
                {
                    if let Some(id) = self.focus_ids.first() {
                        ctx.memory_mut(|memory| memory.request_focus(*id));
                    }
                }
            }
        }
        self.keyboard_active = enabled
            && ctx.input(|input| input.focused)
            && ctx.memory(|memory| {
                memory
                    .focused()
                    .is_some_and(|id| self.focus_ids.contains(&id))
            });
        if !open || cancel {
            self.close(ctx);
        } else if enabled && reload {
            self.draft = Some(Draft::from_palette(palette));
            self.error = None;
        } else if enabled && apply && !stale {
            let draft = self.draft.as_ref().unwrap();
            match palette.replace_presets(draft.presets.clone(), draft.active, draft.next) {
                Ok(changed) => {
                    self.close(ctx);
                    return changed;
                }
                Err(error) => self.error = Some(error),
            }
        }
        false
    }

    fn close(&mut self, ctx: &Context) {
        if let Some(id) = ctx
            .memory(|memory| memory.focused())
            .filter(|id| self.focus_ids.contains(id))
        {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        self.open = false;
        self.draft = None;
        self.error = None;
        self.focus_ids.clear();
        self.window_rect = None;
        self.window_layer = None;
        self.keyboard_active = false;
    }
}

fn editor_contents(ui: &mut Ui, draft: &mut Draft, ids: &mut Vec<Id>) {
    ui.horizontal(|ui| {
        ui.strong("Palette");
        let response = egui::ComboBox::from_id_salt("cable-preset")
            .selected_text(&draft.presets[draft.active].name)
            .width(200.0)
            .show_ui(ui, |ui| {
                for (index, preset) in draft.presets.iter().enumerate() {
                    let response = ui.selectable_label(draft.active == index, &preset.name);
                    if response.clicked() {
                        draft.active = index;
                        draft.next = draft.next.min(preset.colors.len() - 1);
                    }
                    ids.push(response.id);
                }
            })
            .response;
        ids.push(response.id);
    });
    ui.horizontal_wrapped(|ui| {
        let response = ui
            .add_enabled(
                draft.presets.len() < MAX_PRESETS,
                egui::Button::new("New from copy"),
            )
            .on_hover_text(format!(
                "Create a palette from this one. Up to {MAX_PRESETS} palettes."
            ));
        if response.clicked() {
            draft.copy_preset();
        }
        ids.push(response.id);
        let response = ui.add_enabled(draft.presets.len() > 1, egui::Button::new("Delete palette"));
        if response.clicked() {
            draft.delete_preset();
        }
        ids.push(response.id);
        let response = ui
            .button("Reset presets")
            .on_hover_text("Restore the built-in presets in this draft. Apply to save.");
        if response.clicked() {
            let generation = draft.generation;
            *draft = Draft::from_palette(&Palette::default());
            draft.generation = generation;
        }
        ids.push(response.id);
    });
    ui.label("Name");
    let response = ui.add(
        egui::TextEdit::singleline(&mut draft.presets[draft.active].name)
            .char_limit(MAX_NAME_CHARS)
            .desired_width(f32::INFINITY),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, ui.is_enabled(), "Palette name")
    });
    ids.push(response.id);
    ui.separator();
    ui.horizontal(|ui| {
        ui.strong("Colors");
        ui.small(format!(
            "{} / {MAX_COLORS}",
            draft.presets[draft.active].colors.len()
        ));
    });
    ui.horizontal_wrapped(|ui| {
        for (index, color) in draft.presets[draft.active]
            .colors
            .iter()
            .copied()
            .enumerate()
        {
            let response = swatch(ui, color, draft.next == index, index);
            if response.clicked() {
                draft.next = index;
            }
            ids.push(response.id);
        }
    });
    let color = &mut draft.presets[draft.active].colors[draft.next];
    ui.horizontal_wrapped(|ui| {
        for (channel, name) in color.iter_mut().zip(["R", "G", "B"]) {
            let response = ui
                .add(
                    egui::DragValue::new(channel)
                        .range(0..=255)
                        .speed(1.0)
                        .prefix(format!("{name} ")),
                )
                .on_hover_text(format!(
                    "{} channel, 0 to 255",
                    match name {
                        "R" => "Red",
                        "G" => "Green",
                        _ => "Blue",
                    }
                ));
            ids.push(response.id);
        }
        ui.monospace(hex(*color));
    });
    ui.horizontal_wrapped(|ui| {
        let count = draft.presets[draft.active].colors.len();
        for (label, forward, can_move) in [
            ("Move left", false, draft.next > 0),
            ("Move right", true, draft.next + 1 < count),
        ] {
            let response = ui.add_enabled(can_move, egui::Button::new(label));
            if response.clicked() {
                draft.move_color(forward);
            }
            ids.push(response.id);
        }
        let response = ui.add_enabled(count < MAX_COLORS, egui::Button::new("Add color"));
        if response.clicked() {
            let colors = &mut draft.presets[draft.active].colors;
            colors.insert(draft.next + 1, colors[draft.next]);
            draft.next += 1;
        }
        ids.push(response.id);
        let response = ui.add_enabled(count > 1, egui::Button::new("Remove color"));
        if response.clicked() {
            let colors = &mut draft.presets[draft.active].colors;
            colors.remove(draft.next);
            draft.next = draft.next.min(colors.len() - 1);
        }
        ids.push(response.id);
    });
    ui.small("Selected color becomes the next cable. Drag RGB values or double-click to type.");
}

fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}
fn hex(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

fn swatch(ui: &mut Ui, color: [u8; 3], selected: bool, index: usize) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
    let label = format!(
        "Color {}: {}{}",
        index + 1,
        hex(color),
        if selected { ", next cable" } else { "" }
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, &label)
    });
    let border = if selected || response.has_focus() {
        AMBER
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    ui.painter().rect_filled(
        rect.shrink(3.0),
        5.0,
        if ui.is_enabled() {
            rgb(color)
        } else {
            rgb(color).gamma_multiply(0.4)
        },
    );
    ui.painter().rect_stroke(
        rect.shrink(0.5),
        6.0,
        Stroke::new(if selected { 2.0_f32 } else { 1.0_f32 }, border),
        egui::StrokeKind::Inside,
    );
    if selected {
        let ink = if u16::from(color[0]) * 3 + u16::from(color[1]) * 6 + u16::from(color[2]) > 1400
        {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        ui.painter().circle_filled(rect.center(), 3.0, ink);
    }
    response.on_hover_text(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, Event, Key, Modifiers, PointerButton, Pos2};

    #[test]
    fn cable_finish_swatch_click_selects_material_once() {
        use crate::cable_presentation::CableFinish;
        let ctx = Context::default();
        let mut finish = CableFinish::Classic;
        let mut frame = |events| {
            let mut changed = false;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(650.0, 200.0))),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        changed = cable_finish_picker(ui, &mut finish);
                    });
                },
            );
            (changed, output)
        };
        let (_, initial) = frame(vec![]);
        let target = text_position(&initial, "\n\nWoven");
        assert!(!frame(vec![Event::PointerMoved(target), pointer(target, true)]).0);
        assert!(frame(vec![pointer(target, false)]).0);
        assert!(!frame(vec![pointer(target, true)]).0);
        assert!(!frame(vec![pointer(target, false)]).0);
        assert_eq!(finish, CableFinish::Woven);
    }

    fn frame(
        ctx: &Context,
        panel: &mut Panel,
        palette: &mut Palette,
        events: Vec<Event>,
        enabled: bool,
    ) -> (bool, egui::FullOutput) {
        let mut changed = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 620.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    changed |= panel.toolbar(ui, palette, enabled);
                });
                changed |= panel.show(ctx, palette, enabled);
            },
        );
        (changed, output)
    }

    fn text_position(output: &egui::FullOutput, needle: &str) -> Pos2 {
        fn search(shape: &egui::Shape, needle: &str) -> Option<Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == needle => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| search(shape, needle)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| search(&shape.shape, needle))
            .unwrap_or_else(|| panic!("Missing UI text: {needle}"))
    }

    fn pointer(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn click(
        ctx: &Context,
        panel: &mut Panel,
        palette: &mut Palette,
        pos: Pos2,
        enabled: bool,
    ) -> bool {
        let first = frame(
            ctx,
            panel,
            palette,
            vec![Event::PointerMoved(pos), pointer(pos, true)],
            enabled,
        )
        .0;
        first
            | frame(
                ctx,
                panel,
                palette,
                vec![Event::PointerMoved(pos), pointer(pos, false)],
                enabled,
            )
            .0
    }

    fn settle(ctx: &Context, panel: &mut Panel, palette: &mut Palette) -> egui::FullOutput {
        frame(ctx, panel, palette, vec![], true);
        frame(ctx, panel, palette, vec![], true).1
    }

    fn escape() -> Event {
        Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn draft_changes_apply_once_and_cancel_preserves_committed_colors() {
        let ctx = Context::default();
        let mut panel = Panel::default();
        let mut palette = Palette::default();
        let initial = palette.clone();
        panel.open();
        let output = settle(&ctx, &mut panel, &mut palette);
        let add = text_position(&output, "Add color");
        assert!(!click(&ctx, &mut panel, &mut palette, add, true));
        assert_eq!(
            palette, initial,
            "draft edit cannot mutate committed settings"
        );
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let apply = text_position(&output, "Apply");
        assert!(click(&ctx, &mut panel, &mut palette, apply, true));
        assert_eq!(
            palette.presets()[0].colors.len(),
            initial.presets()[0].colors.len() + 1
        );
        assert_eq!(palette.next_index(), 1);
        assert!(!panel.is_open());
        assert!(!frame(&ctx, &mut panel, &mut palette, vec![], true).0);
        let committed = palette.clone();
        panel.open();
        let output = settle(&ctx, &mut panel, &mut palette);
        let remove = text_position(&output, "Remove color");
        assert!(!click(&ctx, &mut panel, &mut palette, remove, true));
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let cancel = text_position(&output, "Cancel");
        assert!(!click(&ctx, &mut panel, &mut palette, cancel, true));
        assert_eq!(palette, committed);
        assert!(!panel.is_open());
    }

    #[test]
    fn external_change_requires_reload_and_disabled_apply_cannot_commit() {
        let ctx = Context::default();
        let mut panel = Panel::default();
        let mut palette = Palette::default();
        panel.open();
        let output = settle(&ctx, &mut panel, &mut palette);
        let apply = text_position(&output, "Apply");
        panel.draft.as_mut().unwrap().presets[0].name = "Unsaved".into();
        assert!(!click(&ctx, &mut panel, &mut palette, apply, false));
        assert_eq!(palette.presets()[0].name, "Studio");
        assert!(panel.is_open());
        palette.cycle(true);
        let external = palette.clone();
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let apply = text_position(&output, "Apply");
        assert!(!click(&ctx, &mut panel, &mut palette, apply, true));
        assert_eq!(palette, external);
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let reload = text_position(&output, "Reload current palette");
        assert!(!click(&ctx, &mut panel, &mut palette, reload, true));
        assert_eq!(panel.draft.as_ref().unwrap().presets, palette.presets());
        assert_eq!(panel.draft.as_ref().unwrap().next, palette.next_index());
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let apply = text_position(&output, "Apply");
        assert!(
            !click(&ctx, &mut panel, &mut palette, apply, true),
            "unchanged Apply must not request persistence"
        );
        assert!(!panel.is_open());
    }

    #[test]
    fn editor_hover_does_not_own_escape_and_outside_press_releases_focus() {
        let ctx = Context::default();
        let mut panel = Panel::default();
        let mut palette = Palette::default();
        panel.open();
        settle(&ctx, &mut panel, &mut palette);
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        frame(&ctx, &mut panel, &mut palette, vec![escape()], true);
        assert!(
            panel.is_open(),
            "unfocused editor must leave rack Escape alone"
        );
        let id = panel.focus_ids[0];
        ctx.memory_mut(|memory| memory.request_focus(id));
        let outside = pos2(780.0, 600.0);
        frame(
            &ctx,
            &mut panel,
            &mut palette,
            vec![
                Event::PointerMoved(outside),
                pointer(outside, true),
                escape(),
            ],
            true,
        );
        assert!(panel.is_open());
        assert!(!ctx.memory(|memory| memory.has_focus(id)));
        frame(
            &ctx,
            &mut panel,
            &mut palette,
            vec![pointer(outside, false)],
            true,
        );
        ctx.memory_mut(|memory| memory.request_focus(id));
        frame(&ctx, &mut panel, &mut palette, vec![], true);
        assert!(panel.keyboard_active());
        frame(&ctx, &mut panel, &mut palette, vec![escape()], true);
        assert!(!panel.is_open(), "owned Escape dismisses the draft");
    }

    #[test]
    fn popup_changes_next_color_without_opening_editor_or_touching_colors() {
        let ctx = Context::default();
        let mut panel = Panel::default();
        let mut palette = Palette::default();
        let presets = palette.presets().to_vec();
        let output = settle(&ctx, &mut panel, &mut palette);
        let chooser = text_position(&output, "Cable");
        assert!(!click(&ctx, &mut panel, &mut palette, chooser, true));
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let next = text_position(&output, "Next");
        assert!(click(&ctx, &mut panel, &mut palette, next, true));
        assert_eq!(palette.next_index(), 1);
        assert_eq!(palette.presets(), presets);
        assert!(!panel.is_open());
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let automatic = text_position(&output, "Advance after each cable");
        assert!(click(&ctx, &mut panel, &mut palette, automatic, true));
        assert!(!palette.auto_advance());
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let preset = text_position(&output, "Palette: Studio");
        assert!(!click(&ctx, &mut panel, &mut palette, preset, true));
        let output = frame(&ctx, &mut panel, &mut palette, vec![], true).1;
        let bright = text_position(&output, "Bright");
        assert!(click(&ctx, &mut panel, &mut palette, bright, true));
        assert_eq!(palette.active_index(), 1);
        assert_eq!(palette.next_index(), 0);
        assert!(!panel.is_open());
    }

    #[test]
    fn draft_copy_delete_and_reorder_respect_limits_and_selected_color() {
        let mut draft = Draft::from_palette(&Palette::default());
        let original = draft.presets[0].colors[0];
        draft.move_color(true);
        assert_eq!(draft.next, 1);
        assert_eq!(draft.presets[0].colors[1], original);
        draft.move_color(false);
        assert_eq!(draft.next, 0);
        for _ in 0..MAX_PRESETS + 2 {
            draft.copy_preset();
        }
        assert_eq!(draft.presets.len(), MAX_PRESETS);
        let names = draft
            .presets
            .iter()
            .map(|preset| &preset.name)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), MAX_PRESETS);
        assert!(draft
            .presets
            .iter()
            .all(|preset| preset.name.chars().count() <= MAX_NAME_CHARS));
        for _ in 0..MAX_PRESETS + 2 {
            draft.delete_preset();
        }
        assert_eq!(draft.presets.len(), 1);
        assert!(draft.next < draft.presets[0].colors.len());

        let mut palette = Palette::default();
        palette
            .replace_presets(
                vec![
                    Preset {
                        name: " A ".into(),
                        colors: vec![[1, 2, 3]],
                    },
                    Preset {
                        name: "a (2) ".into(),
                        colors: vec![[4, 5, 6]],
                    },
                ],
                0,
                0,
            )
            .unwrap();
        let mut draft = Draft::from_palette(&palette);
        draft.copy_preset();
        assert_eq!(draft.presets[draft.active].name, "A (3)");
        assert!(
            palette
                .replace_presets(draft.presets, draft.active, draft.next)
                .unwrap(),
            "copying a valid palette must produce a draft the model accepts"
        );
    }

    #[test]
    fn editor_stays_inside_small_viewport() {
        let ctx = Context::default();
        let mut panel = Panel::default();
        let mut palette = Palette::default();
        panel.open();
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(480.0, 400.0));
        for _ in 0..3 {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    assert!(!panel.show(ctx, &mut palette, true));
                },
            );
        }
        assert!(
            viewport.contains_rect(panel.window_rect.unwrap()),
            "editor must remain usable on smaller screens"
        );
    }
}
