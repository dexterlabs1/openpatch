//! Docked, keyboard-accessible module library. The catalog remains the source
//! of truth for module names, controls, ports, and engine requirements.
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Vec2};
use serde_json::Value;
use std::collections::BTreeSet;

const PANEL: Color32 = Color32::from_rgb(35, 38, 39);
const PANEL_TEXT: Color32 = Color32::from_rgb(233, 229, 217);
const PANEL_MUTED: Color32 = Color32::from_rgb(157, 158, 151);
const METAL: Color32 = Color32::from_rgb(218, 214, 202);
const METAL_INK: Color32 = Color32::from_rgb(39, 39, 36);
const AMBER: Color32 = Color32::from_rgb(226, 175, 88);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryAction {
    Add(String),
    EnablePolyphony,
}

/// Drag payload accepted by the rack. The library never commits an add on drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryDrag {
    pub kind: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Category {
    #[default]
    All,
    Favorites,
    Oscillators,
    Filters,
    Envelopes,
    Sequencers,
    Modulation,
    Mixing,
    Routing,
    Utilities,
    InputOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusKind {
    Search,
    Tile,
    Other,
}

impl Category {
    const ALL: [Self; 11] = [
        Self::All,
        Self::Favorites,
        Self::Oscillators,
        Self::Filters,
        Self::Envelopes,
        Self::Sequencers,
        Self::Modulation,
        Self::Mixing,
        Self::Routing,
        Self::Utilities,
        Self::InputOutput,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Favorites => "Favorites",
            Self::Oscillators => "Oscillators",
            Self::Filters => "Filters",
            Self::Envelopes => "Envelopes",
            Self::Sequencers => "Sequencers",
            Self::Modulation => "Modulation",
            Self::Mixing => "Mixing",
            Self::Routing => "Routing",
            Self::Utilities => "Utilities",
            Self::InputOutput => "I/O",
        }
    }

    fn of_kind(kind: &str) -> Self {
        match kind {
            "oscillator" | "analog_vco" | "multi_vco" | "white_noise" => Self::Oscillators,
            "lowpass" | "ladder_vcf" => Self::Filters,
            "adsr" => Self::Envelopes,
            "seq3" => Self::Sequencers,
            "lfo" | "multi_lfo" | "gate" => Self::Modulation,
            "gain" | "mixer" | "analog_vca" | "mix6" | "cv_mix" | "vca_mix" | "fade"
            | "crossfade" => Self::Mixing,
            "mult" | "mid_side" | "octave" | "poly_split" | "poly_merge" | "poly_sum"
            | "switch_1to4" | "switch_4to1" => Self::Routing,
            "output" | "audio_out" | "audio_stereo" | "midi_cv" => Self::InputOutput,
            "pulses" | "push" | "gates" => Self::Utilities,
            _ => Self::Utilities,
        }
    }
}

#[derive(Default)]
pub struct ModuleLibrary {
    search: String,
    category: Category,
    favorites: BTreeSet<String>,
    favorites_changed: bool,
    focus_search: bool,
    highlighted: Option<String>,
    focused_last_frame: Option<FocusKind>,
    scroll_to_kind: Option<String>,
}

impl ModuleLibrary {
    pub fn request_focus(&mut self) {
        self.focus_search = true;
        self.focused_last_frame = None;
    }

    pub fn favorites(&self) -> Vec<String> {
        self.favorites.iter().cloned().collect()
    }

    /// Restore settings without marking them as an edit. Unknown kinds are discarded.
    pub fn set_favorites(&mut self, kinds: &[String], catalog: &Value) {
        let known: BTreeSet<&str> = modules(catalog)
            .filter_map(|module| module["kind"].as_str())
            .collect();
        self.favorites = kinds
            .iter()
            .filter(|kind| known.contains(kind.as_str()))
            .cloned()
            .collect();
        self.favorites_changed = false;
    }

    pub fn take_favorites_changed(&mut self) -> bool {
        std::mem::take(&mut self.favorites_changed)
    }

    fn toggle_favorite(&mut self, kind: &str) {
        if !self.favorites.remove(kind) {
            self.favorites.insert(kind.to_owned());
        }
        self.favorites_changed = true;
    }

    fn matches(&self, module: &Value, query: &str) -> bool {
        let Some(kind) = module["kind"].as_str() else {
            return false;
        };
        if self.category == Category::Favorites && !self.favorites.contains(kind) {
            return false;
        }
        if !matches!(self.category, Category::All | Category::Favorites)
            && Category::of_kind(kind) != self.category
        {
            return false;
        }
        if query.is_empty() {
            return true;
        }
        [
            module["kind"].as_str(),
            module["title"].as_str(),
            module["description"].as_str(),
            module["model"].as_str(),
        ]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(query))
            || module["tags"].as_array().is_some_and(|tags| {
                tags.iter()
                    .filter_map(Value::as_str)
                    .any(|tag| tag.to_lowercase().contains(query))
            })
            || module["inputs"].as_array().is_some_and(|ports| {
                ports
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|port| port.to_lowercase().contains(query))
            })
            || module["outputs"].as_array().is_some_and(|ports| {
                ports
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|port| port.to_lowercase().contains(query))
            })
            || module["params"].as_object().is_some_and(|params| {
                params
                    .keys()
                    .any(|name| name.to_lowercase().contains(query))
            })
    }

    fn filtered<'a>(&self, catalog: &'a Value) -> Vec<&'a Value> {
        let query = self.search.trim().to_lowercase();
        modules(catalog)
            .filter(|module| self.matches(module, &query))
            .collect()
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        catalog: &Value,
        patch_version: u32,
        enabled: bool,
    ) -> Option<LibraryAction> {
        if !*open {
            self.focused_last_frame = None;
            return None;
        }
        if self.focus_search && enabled {
            // Focus before the TextEdit is drawn so it installs its arrow-key
            // focus filter in this first pass. A post-widget request loses
            // ArrowDown to egui's spatial focus navigation on the next pass.
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("module-library-search")));
            self.focus_search = false;
        }
        let previous_focus = self.focused_last_frame;
        let keys = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Space),
                input.key_pressed(egui::Key::Escape),
                input.key_pressed(egui::Key::ArrowLeft),
                input.key_pressed(egui::Key::ArrowRight),
                input.key_pressed(egui::Key::ArrowUp),
                input.key_pressed(egui::Key::ArrowDown),
            )
        });
        let mut action = None;
        let mut close = false;
        let mut search_id = None;
        let mut other_ids = Vec::new();
        let mut tile_ids = Vec::new();
        let mut visible_kinds = Vec::new();
        let panel = egui::SidePanel::right("module-library")
            .default_width(310.0)
            .width_range(260.0..=440.0)
            .resizable(true)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 12)),
            )
            .show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    ui.visuals_mut().override_text_color = Some(PANEL_TEXT);
                    ui.horizontal(|ui| {
                        ui.heading("Library");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button = ui.button("×").on_hover_text("Close library");
                            other_ids.push(button.id);
                            if button.clicked() {
                                close = true;
                            }
                        });
                    });
                    let search = ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .id(egui::Id::new("module-library-search"))
                            .hint_text("Search modules, controls, or ports")
                            .desired_width(f32::INFINITY),
                    );
                    search_id = Some(search.id);
                    ui.add_space(6.0);
                    let mut category_changed = false;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
                        for category in Category::ALL {
                            let selected = category == self.category;
                            let label = if category == Category::Favorites {
                                "★ Favorites"
                            } else {
                                category.label()
                            };
                            let fill = if selected {
                                Color32::from_rgb(212, 165, 87)
                            } else {
                                Color32::from_rgb(52, 56, 58)
                            };
                            let button = ui.add(
                                egui::Button::new(
                                    egui::RichText::new(label).size(11.0).color(if selected {
                                        METAL_INK
                                    } else {
                                        PANEL_TEXT
                                    }),
                                )
                                .fill(fill)
                                .corner_radius(11.0),
                            );
                            other_ids.push(button.id);
                            if button.clicked() {
                                self.category = category;
                                category_changed = true;
                            }
                        }
                    });
                    if category_changed {
                        search.request_focus();
                    }
                    ui.add_space(5.0);
                    if patch_version < 2 {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new("Polyphonic modules need engine v2.")
                                    .size(10.0)
                                    .color(PANEL_MUTED),
                            );
                            let button = ui.small_button("Enable polyphony…");
                            other_ids.push(button.id);
                            if button.clicked() {
                                action = Some(LibraryAction::EnablePolyphony);
                            }
                        });
                    }
                    let visible = self.filtered(catalog);
                    if !visible
                        .iter()
                        .any(|module| module["kind"].as_str() == self.highlighted.as_deref())
                    {
                        self.highlighted = visible
                            .first()
                            .and_then(|module| module["kind"].as_str())
                            .map(str::to_owned);
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} module{}",
                            visible.len(),
                            if visible.len() == 1 { "" } else { "s" }
                        ))
                        .size(10.0)
                        .color(PANEL_MUTED),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if visible.is_empty() {
                                ui.add_space(22.0);
                                ui.label(
                                    egui::RichText::new(
                                        if self.category == Category::Favorites
                                            && self.favorites.is_empty()
                                        {
                                            "Star modules to keep them here."
                                        } else {
                                            "No modules match this search."
                                        },
                                    )
                                    .color(PANEL_MUTED),
                                );
                                return;
                            }
                            let gap = 8.0;
                            let tile_width = ((ui.available_width() - gap) * 0.5).max(108.0);
                            for row in visible.chunks(2) {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = gap;
                                    for module in row {
                                        if let Some(kind) = module["kind"].as_str() {
                                            visible_kinds.push(kind);
                                            let (tile_action, id, star_id) =
                                                self.tile(ui, module, tile_width, patch_version);
                                            tile_ids.push((kind, id));
                                            other_ids.push(star_id);
                                            if tile_action.is_some() {
                                                action = tile_action;
                                            }
                                        }
                                    }
                                });
                                ui.add_space(gap);
                            }
                        });
                });
            });
        if !enabled {
            self.focused_last_frame = None;
            return None;
        }
        let outside_press = ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| !panel.response.rect.contains(pos))
        });
        let focused_id = ctx.memory(|memory| memory.focused());
        let focus = focused_id.and_then(|id| {
            if search_id == Some(id) {
                Some(FocusKind::Search)
            } else if tile_ids.iter().any(|(_, tile_id)| *tile_id == id) {
                Some(FocusKind::Tile)
            } else if other_ids.contains(&id) {
                Some(FocusKind::Other)
            } else {
                None
            }
        });
        let window_focused = ctx.input(|input| input.focused);
        let previous_focus = if outside_press || !window_focused {
            None
        } else {
            previous_focus
        };
        let owns_keys =
            window_focused && !outside_press && (previous_focus.is_some() || focus.is_some());
        if owns_keys {
            for (pressed, key) in [
                (keys.0, egui::Key::Enter),
                (keys.1, egui::Key::Space),
                (keys.2, egui::Key::Escape),
            ] {
                if pressed {
                    consume_key(ctx, key);
                }
            }
        }
        if close {
            *open = false;
            self.focused_last_frame = None;
            return None;
        }
        if !owns_keys {
            self.focused_last_frame = if outside_press || !window_focused {
                None
            } else {
                focus
            };
            return action;
        }
        if keys.2 {
            *open = false;
            self.focused_last_frame = None;
            return None;
        }
        let tile_has_focus =
            focus == Some(FocusKind::Tile) || previous_focus == Some(FocusKind::Tile);
        let step = if keys.5 || (tile_has_focus && keys.3) {
            -1
        } else if keys.6 || (tile_has_focus && keys.4) {
            1
        } else {
            0
        };
        if step != 0 {
            // egui computes spatial focus movement before widgets run. This
            // panel handles these arrows itself, including when no tile matches.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
            if !visible_kinds.is_empty() {
                let current = visible_kinds
                    .iter()
                    .position(|kind| Some(*kind) == self.highlighted.as_deref())
                    .unwrap_or(0);
                let next = if step < 0 {
                    current.saturating_sub(1)
                } else {
                    (current + 1).min(visible_kinds.len() - 1)
                };
                self.highlighted = Some(visible_kinds[next].to_owned());
                self.scroll_to_kind = Some(visible_kinds[next].to_owned());
                if tile_has_focus {
                    if let Some((_, id)) = tile_ids.get(next) {
                        ctx.memory_mut(|memory| memory.request_focus(*id));
                    }
                }
            }
            consume_key(
                ctx,
                if keys.5 {
                    egui::Key::ArrowUp
                } else if keys.6 {
                    egui::Key::ArrowDown
                } else if keys.3 {
                    egui::Key::ArrowLeft
                } else {
                    egui::Key::ArrowRight
                },
            );
        }
        if keys.0
            && action.is_none()
            && focus != Some(FocusKind::Other)
            && previous_focus != Some(FocusKind::Other)
        {
            if let Some(kind) = self.highlighted.as_deref() {
                if let Some(module) =
                    modules(catalog).find(|module| module["kind"].as_str() == Some(kind))
                {
                    action = Some(if available(module, patch_version) {
                        LibraryAction::Add(kind.to_owned())
                    } else {
                        LibraryAction::EnablePolyphony
                    });
                }
            }
        }
        if keys.0 && previous_focus == Some(FocusKind::Search) {
            if let Some(id) = search_id {
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
        }
        let focused_id = ctx.memory(|memory| memory.focused());
        self.focused_last_frame = focused_id.and_then(|id| {
            if search_id == Some(id) {
                Some(FocusKind::Search)
            } else if tile_ids.iter().any(|(_, tile_id)| *tile_id == id) {
                Some(FocusKind::Tile)
            } else if other_ids.contains(&id) {
                Some(FocusKind::Other)
            } else {
                None
            }
        });
        action
    }

    fn tile(
        &mut self,
        ui: &mut egui::Ui,
        module: &Value,
        width: f32,
        patch_version: u32,
    ) -> (Option<LibraryAction>, egui::Id, egui::Id) {
        let kind = module["kind"].as_str().unwrap_or("");
        let title = module["title"].as_str().unwrap_or(kind);
        let supported = available(module, patch_version);
        let (_, rect) = ui.allocate_space(vec2(width, 151.0));
        let id = ui.make_persistent_id(("module-library-tile", kind));
        let response = ui.interact(rect, id, Sense::click_and_drag());
        if self.scroll_to_kind.as_deref() == Some(kind) {
            response.scroll_to_me(None);
            self.scroll_to_kind = None;
        }
        let enabled = ui.is_enabled();
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                if supported {
                    format!("Add {title} module")
                } else {
                    format!("Enable polyphony to add {title} module")
                },
            )
        });
        paint_tile(
            ui.painter(),
            rect,
            module,
            self.highlighted.as_deref() == Some(kind),
            response.hovered(),
            supported,
        );
        let star_rect =
            Rect::from_center_size(rect.right_top() + vec2(-15.0, 16.0), Vec2::splat(25.0));
        let star = ui.interact(star_rect, id.with("favorite"), Sense::click());
        let favorite = self.favorites.contains(kind);
        ui.painter().text(
            star_rect.center(),
            Align2::CENTER_CENTER,
            if favorite { "★" } else { "☆" },
            FontId::proportional(17.0),
            if favorite {
                AMBER
            } else {
                Color32::from_rgb(103, 99, 88)
            },
        );
        let star = star.on_hover_text(if favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        });
        let mut action = None;
        if enabled && star.clicked() {
            self.toggle_favorite(kind);
        } else if enabled
            && response.clicked()
            && !star_rect.contains(
                ui.input(|input| input.pointer.interact_pos().unwrap_or(pos2(-1.0, -1.0))),
            )
        {
            self.highlighted = Some(kind.to_owned());
            response.request_focus();
            action = Some(if supported {
                LibraryAction::Add(kind.to_owned())
            } else {
                LibraryAction::EnablePolyphony
            });
        }
        if enabled && supported && response.drag_started() {
            response.dnd_set_drag_payload(LibraryDrag {
                kind: kind.to_owned(),
            });
        }
        let tooltip = format!(
            "{title} · {}\n{}\nImplementation: {}{}",
            Category::of_kind(kind).label(),
            module["description"]
                .as_str()
                .unwrap_or("Built-in OpenPatch module"),
            module["model"].as_str().unwrap_or("legacy"),
            if supported {
                ""
            } else {
                "\nRequires engine v2"
            },
        );
        response.on_hover_text(tooltip);
        (action, id, star.id)
    }
}

fn modules(catalog: &Value) -> impl Iterator<Item = &Value> {
    catalog["modules"].as_array().into_iter().flatten()
}

fn available(module: &Value, patch_version: u32) -> bool {
    module["min_patch_version"].as_u64().unwrap_or(1) <= u64::from(patch_version)
}

fn consume_key(ctx: &egui::Context, key: egui::Key) {
    ctx.input_mut(|input| {
        let modifiers = input.modifiers;
        input.consume_key(modifiers, key);
    });
}

fn paint_tile(
    painter: &egui::Painter,
    rect: Rect,
    module: &Value,
    highlighted: bool,
    hovered: bool,
    supported: bool,
) {
    let kind = module["kind"].as_str().unwrap_or("");
    let title = module["title"].as_str().unwrap_or(kind);
    let face = rect.shrink(2.0);
    painter.rect_filled(
        rect.translate(vec2(1.0, 2.0)),
        3.0,
        Color32::from_black_alpha(85),
    );
    painter.rect_filled(rect, 3.0, Color32::from_rgb(85, 82, 74));
    painter.rect_filled(
        face,
        2.0,
        if hovered {
            Color32::from_rgb(235, 229, 213)
        } else {
            METAL
        },
    );
    painter.line_segment(
        [face.left_top(), face.right_top()],
        Stroke::new(1.0_f32, Color32::from_rgb(249, 244, 230)),
    );
    painter.line_segment(
        [face.right_top(), face.right_bottom()],
        Stroke::new(1.0_f32, Color32::from_rgb(109, 104, 92)),
    );
    for grain in 0..4 {
        let y = face.top() + 40.0 + grain as f32 * 27.0;
        painter.line_segment(
            [pos2(face.left() + 4.0, y), pos2(face.right() - 4.0, y)],
            Stroke::new(0.6_f32, Color32::from_white_alpha(14)),
        );
    }
    if highlighted {
        painter.rect_stroke(
            rect.shrink(1.0),
            2.0,
            Stroke::new(1.5_f32, AMBER),
            egui::StrokeKind::Inside,
        );
    }
    for center in [
        face.left_top() + vec2(7.0, 7.0),
        face.left_bottom() + vec2(7.0, -7.0),
        face.right_bottom() + vec2(-7.0, -7.0),
    ] {
        painter.circle_filled(center, 2.0, Color32::from_rgb(121, 117, 108));
        painter.circle_filled(
            center - vec2(0.3, 0.4),
            1.2,
            Color32::from_rgb(232, 228, 216),
        );
    }
    let mut job = egui::text::LayoutJob::simple(
        title.to_owned(),
        FontId::proportional(12.0),
        METAL_INK,
        face.width() - 42.0,
    );
    job.wrap.max_rows = 1;
    painter.galley(
        face.min + vec2(15.0, 12.0),
        painter.layout_job(job),
        METAL_INK,
    );
    let params = module["params"]
        .as_object()
        .map_or(0, |params| params.len());
    let inputs = module["inputs"].as_array().map_or(0, Vec::len);
    let outputs = module["outputs"].as_array().map_or(1, Vec::len);
    let area = Rect::from_min_max(face.min + vec2(12.0, 37.0), face.max - vec2(12.0, 26.0));
    if kind == "seq3" && params >= 8 {
        for step in 0..8 {
            let x = area.left() + area.width() * (step as f32 + 0.5) / 8.0;
            painter.circle_filled(
                pos2(x, area.top() + 13.0),
                2.8,
                if step == 0 {
                    AMBER
                } else {
                    Color32::from_rgb(135, 133, 124)
                },
            );
            mini_knob(painter, pos2(x, area.top() + 34.0), 4.8);
        }
    } else if kind == "push" {
        for (index, x) in [area.left() + 28.0, area.right() - 28.0]
            .into_iter()
            .enumerate()
        {
            painter.rect_filled(
                Rect::from_center_size(pos2(x, area.center().y - 5.0), vec2(28.0, 22.0)),
                3.0,
                if index == 0 {
                    AMBER
                } else {
                    Color32::from_rgb(163, 158, 143)
                },
            );
            painter.circle_filled(pos2(x, area.center().y + 17.0), 3.5, AMBER);
        }
    } else if kind == "gates" {
        mini_knob(
            painter,
            pos2(area.left() + 27.0, area.center().y - 8.0),
            13.0,
        );
        painter.rect_filled(
            Rect::from_center_size(
                pos2(area.right() - 27.0, area.center().y - 8.0),
                vec2(32.0, 20.0),
            ),
            3.0,
            Color32::from_rgb(163, 158, 143),
        );
        for index in 0..6 {
            let x = area.left() + 24.0 + (index % 3) as f32 * (area.width() - 48.0) / 2.0;
            let y = area.bottom() - 22.0 + (index / 3) as f32 * 14.0;
            mini_jack(painter, pos2(x, y));
        }
    } else if kind == "pulses" {
        for row in 0..4 {
            let y = area.top() + 7.0 + row as f32 * 16.0;
            painter.rect_filled(
                Rect::from_center_size(pos2(area.left() + 21.0, y), vec2(20.0, 8.0)),
                2.0,
                if row == 0 {
                    AMBER
                } else {
                    Color32::from_rgb(150, 146, 135)
                },
            );
            mini_jack(painter, pos2(area.center().x + 8.0, y));
            mini_jack(painter, pos2(area.right() - 10.0, y));
        }
    } else if params == 0 {
        let y = area.center().y;
        let left = pos2(area.left() + 17.0, y);
        let right = pos2(area.right() - 17.0, y);
        painter.line_segment(
            [left, right],
            Stroke::new(1.1_f32, Color32::from_rgb(113, 110, 99)),
        );
        mini_jack(painter, left);
        mini_jack(painter, right);
    } else {
        let count = params.min(4);
        for index in 0..count {
            let columns = if count == 1 { 1 } else { 2 };
            let rows = count.div_ceil(columns);
            let column = index % columns;
            let row = index / columns;
            let x = area.left() + area.width() * (column as f32 + 0.5) / columns as f32;
            let y = area.top() + area.height() * (row as f32 + 0.5) / rows as f32;
            mini_knob(painter, pos2(x, y), if count == 1 { 18.0 } else { 11.0 });
        }
    }
    let port_y = face.bottom() - 18.0;
    for index in 0..inputs.min(3) {
        mini_jack(
            painter,
            pos2(face.left() + 18.0 + index as f32 * 13.0, port_y),
        );
    }
    for index in 0..outputs.min(3) {
        mini_jack(
            painter,
            pos2(face.right() - 18.0 - index as f32 * 13.0, port_y),
        );
    }
    if !supported {
        painter.rect_filled(face, 2.0, Color32::from_black_alpha(105));
        painter.text(
            face.center_bottom() - vec2(0.0, 17.0),
            Align2::CENTER_CENTER,
            "ENGINE V2",
            FontId::proportional(10.0),
            PANEL_TEXT,
        );
    }
}

fn mini_knob(painter: &egui::Painter, center: egui::Pos2, radius: f32) {
    painter.circle_filled(
        center + vec2(0.7, 1.4),
        radius + 1.5,
        Color32::from_black_alpha(65),
    );
    painter.circle_filled(center, radius + 0.6, Color32::from_rgb(105, 104, 98));
    painter.circle_filled(center, radius * 0.81, Color32::from_rgb(35, 38, 39));
    painter.line_segment(
        [
            center - vec2(0.0, radius * 0.32),
            center - vec2(0.0, radius * 0.7),
        ],
        Stroke::new(1.3_f32, Color32::from_rgb(238, 234, 222)),
    );
}

fn mini_jack(painter: &egui::Painter, center: egui::Pos2) {
    painter.circle_filled(center + vec2(0.4, 0.8), 5.5, Color32::from_black_alpha(69));
    painter.circle_filled(center, 5.0, Color32::from_rgb(127, 123, 112));
    painter.circle_filled(center, 3.7, Color32::from_rgb(226, 223, 211));
    painter.circle_filled(center, 2.5, Color32::from_rgb(28, 30, 30));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({"modules":[
            {"kind":"analog_vco","title":"VCO","model":"idealized","description":"Audio oscillator","inputs":["pitch"],"outputs":["saw"],"params":{"frequency":{"default":220}}},
            {"kind":"ladder_vcf","title":"LADDER","model":"circuit-derived","description":"Resonant filter","inputs":["signal"],"outputs":["out"],"params":{"cutoff":{"default":1200}}},
            {"kind":"poly_split","title":"SPLIT","model":"functional","description":"Channel routing","tags":["polyphony"],"min_patch_version":2,"inputs":["signal"],"outputs":["out1","out2"],"params":{}}
        ]})
    }

    fn frame(
        ctx: &egui::Context,
        library: &mut ModuleLibrary,
        open: &mut bool,
        catalog: &Value,
        time: f64,
        events: Vec<egui::Event>,
        enabled: bool,
    ) -> (Option<LibraryAction>, egui::FullOutput) {
        let mut action = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                time: Some(time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| {
                action = library.show(ctx, open, catalog, 2, enabled);
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        (action, output)
    }

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn text_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        fn in_shape(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| in_shape(shape, label)),
                _ => None,
            }
        }
        output
            .shapes
            .iter()
            .find_map(|shape| in_shape(&shape.shape, label))
            .unwrap_or_else(|| panic!("missing library label {label}"))
    }

    #[test]
    fn search_categories_and_favorites_use_catalog_meaning_not_model_provenance() {
        let catalog = fixture();
        let mut library = ModuleLibrary {
            search: "cutoff".into(),
            ..Default::default()
        };
        assert_eq!(
            library
                .filtered(&catalog)
                .iter()
                .map(|module| module["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["ladder_vcf"]
        );
        library.search = "saw".into();
        assert_eq!(
            library.filtered(&catalog).len(),
            1,
            "output ports are searchable"
        );
        library.search = "polyphony".into();
        assert_eq!(library.filtered(&catalog)[0]["kind"], "poly_split");
        library.search.clear();
        library.category = Category::Oscillators;
        assert_eq!(library.filtered(&catalog)[0]["kind"], "analog_vco");
        assert_eq!(Category::of_kind("ladder_vcf"), Category::Filters);
        assert_eq!(Category::of_kind("poly_split"), Category::Routing);
        assert_eq!(Category::of_kind("pulses"), Category::Utilities);
        assert_eq!(Category::of_kind("push"), Category::Utilities);
        library.set_favorites(&["analog_vco".into(), "removed_plugin".into()], &catalog);
        assert_eq!(library.favorites(), vec!["analog_vco"]);
        assert!(!library.take_favorites_changed());
        library.category = Category::Favorites;
        assert_eq!(library.filtered(&catalog).len(), 1);
        library.toggle_favorite("ladder_vcf");
        assert_eq!(library.filtered(&catalog).len(), 2);
        assert!(library.take_favorites_changed());
        assert!(!library.take_favorites_changed());
    }

    #[test]
    fn search_focus_arrow_enter_and_escape_work_without_closing_on_add() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.request_focus();
        frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        assert_eq!(library.highlighted.as_deref(), Some("analog_vco"));
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            1.0,
            vec![key(egui::Key::ArrowDown, true)],
            true,
        );
        assert_eq!(action, None);
        assert_eq!(library.highlighted.as_deref(), Some("ladder_vcf"));
        frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            1.1,
            vec![key(egui::Key::ArrowDown, false)],
            true,
        );
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            2.0,
            vec![key(egui::Key::Enter, true)],
            true,
        );
        assert_eq!(action, Some(LibraryAction::Add("ladder_vcf".into())));
        assert!(open);
        frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            2.1,
            vec![key(egui::Key::Enter, false)],
            true,
        );
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            3.0,
            vec![key(egui::Key::Escape, true)],
            true,
        );
        assert_eq!(action, None);
        assert!(!open);
    }

    #[test]
    fn focused_library_consumes_activation_and_close_keys_before_global_shortcuts() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.request_focus();
        frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        for (time, pressed_key, expected_action, expected_open) in [
            (1.0, egui::Key::Space, None, true),
            (
                2.0,
                egui::Key::Enter,
                Some(LibraryAction::Add("analog_vco".into())),
                true,
            ),
            (3.0, egui::Key::Escape, None, false),
        ] {
            let mut action = None;
            let mut reaches_global_shortcuts = false;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                    time: Some(time),
                    focused: true,
                    events: vec![key(pressed_key, true)],
                    ..Default::default()
                },
                |ctx| {
                    action = library.show(ctx, &mut open, &catalog, 2, true);
                    reaches_global_shortcuts = ctx.input(|input| input.key_pressed(pressed_key));
                    egui::CentralPanel::default().show(ctx, |_ui| {});
                },
            );
            assert_eq!(action, expected_action);
            assert_eq!(open, expected_open);
            assert!(
                !reaches_global_shortcuts,
                "{pressed_key:?} escaped the library"
            );
        }
    }

    #[test]
    fn outside_pointer_press_releases_keyboard_ownership() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.request_focus();
        frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        let outside = pos2(100.0, 100.0);
        let mut action = None;
        let mut global_sees_enter = false;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                time: Some(1.0),
                focused: true,
                events: vec![
                    egui::Event::PointerMoved(outside),
                    egui::Event::PointerButton {
                        pos: outside,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    key(egui::Key::Enter, true),
                ],
                ..Default::default()
            },
            |ctx| {
                action = library.show(ctx, &mut open, &catalog, 2, true);
                global_sees_enter = ctx.input(|input| input.key_pressed(egui::Key::Enter));
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        assert_eq!(action, None);
        assert!(global_sees_enter);
        assert_eq!(library.focused_last_frame, None);
    }

    #[test]
    fn star_click_updates_favorites_without_adding_and_disabled_panel_ignores_keys() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.request_focus();
        let (_, output) = frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        let star = text_center(&output, "☆");
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            let (action, _) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                time,
                vec![
                    egui::Event::PointerMoved(star),
                    egui::Event::PointerButton {
                        pos: star,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
            assert_eq!(action, None);
        }
        assert_eq!(library.favorites(), vec!["analog_vco"]);
        assert!(library.take_favorites_changed());
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            2.0,
            vec![key(egui::Key::Enter, true)],
            false,
        );
        assert_eq!(action, None);
        assert!(open);
        assert!(!library.take_favorites_changed());
    }

    #[test]
    fn version_one_keyboard_requests_upgrade_without_adding() {
        let catalog = fixture();
        assert!(available(&catalog["modules"][0], 1));
        assert!(!available(&catalog["modules"][2], 1));
        let ctx = egui::Context::default();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.search = "poly_split".into();
        library.request_focus();
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                time: Some(0.0),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                action = library.show(ctx, &mut open, &catalog, 1, true);
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        assert_eq!(action, None);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                time: Some(1.0),
                focused: true,
                events: vec![key(egui::Key::Enter, true)],
                ..Default::default()
            },
            |ctx| {
                action = library.show(ctx, &mut open, &catalog, 1, true);
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        assert_eq!(action, Some(LibraryAction::EnablePolyphony));
        assert!(open);
    }

    #[test]
    fn version_one_click_on_unsupported_tile_requests_upgrade() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        let mut action = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                time: Some(0.0),
                focused: true,
                ..Default::default()
            },
            |ctx| {
                action = library.show(ctx, &mut open, &catalog, 1, true);
                egui::CentralPanel::default().show(ctx, |_ui| {});
            },
        );
        assert_eq!(action, None);
        let target = text_center(&output, "SPLIT");
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                    time: Some(time),
                    focused: true,
                    events: vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    action = library.show(ctx, &mut open, &catalog, 1, true);
                    egui::CentralPanel::default().show(ctx, |_ui| {});
                },
            );
        }
        assert_eq!(action, Some(LibraryAction::EnablePolyphony));
        assert!(open);
    }

    #[test]
    fn tile_drag_exposes_payload_without_click_add_and_disabled_panel_has_no_payload() {
        fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }
        let catalog = fixture();
        for enabled in [true, false] {
            let ctx = egui::Context::default();
            let mut library = ModuleLibrary::default();
            let mut open = true;
            let (_, output) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                0.0,
                vec![],
                enabled,
            );
            let start = text_center(&output, "VCO");
            let (action, _) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                1.0,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
                enabled,
            );
            assert_eq!(action, None);
            let (action, _) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                1.1,
                vec![egui::Event::PointerMoved(start + vec2(35.0, 24.0))],
                enabled,
            );
            assert_eq!(action, None);
            assert_eq!(
                egui::DragAndDrop::payload::<LibraryDrag>(&ctx).map(|payload| payload.kind.clone()),
                enabled.then_some("analog_vco".to_owned())
            );
            frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                1.2,
                vec![pointer(start + vec2(35.0, 24.0), false)],
                enabled,
            );
        }
    }
}
