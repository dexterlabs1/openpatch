//! Docked, keyboard-accessible module library. The catalog remains the source
//! of truth for module names, controls, ports, and engine requirements.
use crate::rack::{ModulePreview, PANEL_HEIGHT};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Vec2};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const PANEL: Color32 = Color32::from_rgb(35, 38, 39);
const PANEL_TEXT: Color32 = Color32::from_rgb(233, 229, 217);
const PANEL_MUTED: Color32 = Color32::from_rgb(157, 158, 151);
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
    Inspection,
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
            "oscillator" | "analog_vco" | "multi_vco" | "white_noise" | "drum_voice"
            | "modal_resonator" | "colored_noise" | "macro_voice" => Self::Oscillators,
            "lowpass" | "ladder_vcf" | "svf_filter" | "stock_vcf" => Self::Filters,
            "adsr" | "adsr_eg" => Self::Envelopes,
            "seq3" | "score_sequencer" => Self::Sequencers,
            "lfo" | "multi_lfo" | "gate" | "random_values" | "sh_asr" => Self::Modulation,
            "gain"
            | "mixer"
            | "analog_vca"
            | "stock_vca"
            | "mix6"
            | "cv_mix"
            | "vca_mix"
            | "fade"
            | "crossfade"
            | "stock_delay"
            | "stereo_delay"
            | "stereo_reverb"
            | "sidechain_compressor" => Self::Mixing,
            "mult" | "mid_side" | "octave" | "poly_split" | "poly_merge" | "poly_sum"
            | "switch_1to4" | "switch_4to1" => Self::Routing,
            "output" | "audio_out" | "audio_stereo" | "midi_cv" | "midi_cc" | "midi_gate" => {
                Self::InputOutput
            }
            "pulses" | "push" | "gates" | "poly_viz" | "scope" => Self::Utilities,
            _ => Self::Utilities,
        }
    }
}

pub struct ModuleLibrary {
    search: String,
    category: Category,
    favorites: BTreeSet<String>,
    favorites_changed: bool,
    focus_search: bool,
    highlighted: Option<String>,
    focused_last_frame: Option<FocusKind>,
    scroll_to_kind: Option<String>,
    previews: BTreeMap<String, ModulePreview>,
    inspected: Option<String>,
    preview_zoom: f32,
    preview_zoom_changed: bool,
}

impl Default for ModuleLibrary {
    fn default() -> Self {
        Self {
            search: String::new(),
            category: Category::default(),
            favorites: BTreeSet::new(),
            favorites_changed: false,
            focus_search: false,
            highlighted: None,
            focused_last_frame: None,
            scroll_to_kind: None,
            previews: BTreeMap::new(),
            inspected: None,
            preview_zoom: 0.5,
            preview_zoom_changed: false,
        }
    }
}

const PREVIEW_ZOOMS: [(f32, &str); 7] = [
    (0.25, "25%"),
    (std::f32::consts::FRAC_1_SQRT_2 * 0.5, "35%"),
    (0.5, "50%"),
    (std::f32::consts::FRAC_1_SQRT_2, "71%"),
    (1.0, "100%"),
    (std::f32::consts::SQRT_2, "141%"),
    (2.0, "200%"),
];

impl ModuleLibrary {
    pub fn preview_zoom(&self) -> f32 {
        self.preview_zoom
    }

    /// Restore/programmatic assignment has no UI edit notification.
    pub fn set_preview_zoom(&mut self, zoom: f32) {
        let zoom = if zoom.is_finite() {
            zoom.clamp(0.25, 2.0)
        } else {
            0.5
        };
        if self.preview_zoom != zoom {
            self.preview_zoom = zoom;
            self.scroll_to_kind = self.highlighted.clone();
        }
    }

    pub fn take_preview_zoom_changed(&mut self) -> bool {
        std::mem::take(&mut self.preview_zoom_changed)
    }

    fn choose_preview_zoom(&mut self, zoom: f32) {
        if self.preview_zoom != zoom {
            self.set_preview_zoom(zoom);
            self.preview_zoom_changed = true;
        }
    }

    /// Pin a catalog module for inspection without inserting it or changing selection.
    /// Used by explicit Inspect controls; unknown/removed kinds are ignored.
    pub fn inspect(&mut self, kind: &str, catalog: &Value) -> bool {
        if modules(catalog).any(|module| module["kind"].as_str() == Some(kind)) {
            self.inspected = Some(kind.to_owned());
            true
        } else {
            false
        }
    }

    pub(crate) fn keyboard_active(&self) -> bool {
        self.focused_last_frame.is_some()
    }

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
            Some(crate::rack::title(kind)),
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
                    .iter()
                    .filter(|(_, spec)| spec["hidden"] != true)
                    .any(|(name, _)| name.to_lowercase().contains(query))
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
                input.key_pressed(egui::Key::I)
                    && (input.modifiers.command || input.modifiers.ctrl)
                    && !input.modifiers.alt,
            )
        });
        let mut action = None;
        let mut close = false;
        let mut search_id = None;
        let mut other_ids = Vec::new();
        let mut tile_ids = Vec::new();
        let mut visible_kinds = Vec::new();
        let mut zoom_popup_was_open = false;
        let mut zoom_popup_active = false;
        let mut zoom_id = None;
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
                        // A filter can invalidate a pending reveal from an
                        // earlier zoom/navigation frame. Reveal its new highlight.
                        self.scroll_to_kind = self.highlighted.clone();
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} module{}",
                                visible.len(),
                                if visible.len() == 1 { "" } else { "s" }
                            ))
                            .size(10.0)
                            .color(PANEL_MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // ComboBox stores Id::new(salt), then scopes that Id.
                            let id =
                                ui.make_persistent_id(egui::Id::new("module-library-preview-zoom"));
                            zoom_id = Some(id);
                            if !enabled {
                                egui::Popup::close_id(ctx, id.with("popup"));
                            }
                            zoom_popup_was_open = egui::ComboBox::is_open(ctx, id);
                            let mut chosen = None;
                            let mut selected_option = None;
                            let response =
                                egui::ComboBox::from_id_salt("module-library-preview-zoom")
                                    .selected_text(format!(
                                        "{}%",
                                        (self.preview_zoom * 100.0).round() as u32
                                    ))
                                    .width(64.0)
                                    .height(220.0)
                                    .show_ui(ui, |ui| {
                                        if !enabled {
                                            ui.disable();
                                        }
                                        for (zoom, label) in PREVIEW_ZOOMS {
                                            let option = ui.selectable_label(
                                                (self.preview_zoom - zoom).abs() < 0.00001,
                                                label,
                                            );
                                            other_ids.push(option.id);
                                            if (self.preview_zoom - zoom).abs() < 0.00001 {
                                                selected_option = Some(option.id);
                                            }
                                            if enabled && option.clicked() {
                                                chosen = Some(zoom);
                                            }
                                        }
                                    })
                                    .response;
                            debug_assert_eq!(
                                id, response.id,
                                "zoom popup ownership must use the actual ComboBox response ID"
                            );
                            other_ids.push(response.id);
                            zoom_popup_active = egui::ComboBox::is_open(ctx, response.id);
                            if response.clicked() {
                                // Keyboard opening starts at the configured
                                // preset so arrows/Tab can enter the menu.
                                if zoom_popup_active && (keys.0 || keys.1) {
                                    if let Some(id) = selected_option {
                                        ctx.memory_mut(|m| m.request_focus(id));
                                    }
                                } else {
                                    response.request_focus();
                                }
                            }
                            if let Some(zoom) = chosen {
                                self.choose_preview_zoom(zoom);
                                // Popup's default CloseOnClick observes pointer
                                // clicks, so keyboard preset activation closes
                                // this owned menu explicitly as well.
                                egui::Popup::close_id(ctx, response.id.with("popup"));
                                zoom_popup_active = false;
                                response.request_focus();
                            }
                            if response.has_focus() {
                                ctx.memory_mut(|m| {
                                    m.set_focus_lock_filter(
                                        response.id,
                                        egui::EventFilter {
                                            horizontal_arrows: !zoom_popup_active,
                                            vertical_arrows: !zoom_popup_active,
                                            ..Default::default()
                                        },
                                    )
                                });
                            }
                            ui.label(egui::RichText::new("Preview").size(10.0).color(PANEL_MUTED));
                        });
                    });
                    ui.separator();
                    // Floating scroll bars never alter the wrapping width as
                    // they appear, avoiding content-width/scrollbar feedback.
                    ui.spacing_mut().scroll.floating = true;
                    ui.spacing_mut().scroll.floating_allocated_width = 0.0;
                    egui::ScrollArea::both()
                        .id_salt("module-library-tiles")
                        .auto_shrink([false, false])
                        .show_viewport(ui, |ui, viewport| {
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
                            // Plan from metadata alone: no offscreen renderer
                            // cache is constructed to discover panel dimensions.
                            let (cards, size) =
                                preview_flow(&visible, self.preview_zoom, viewport.width());
                            let (content, _) = ui.allocate_exact_size(size, Sense::hover());
                            for (module, card) in visible.iter().zip(cards) {
                                if let Some(kind) = module["kind"].as_str() {
                                    visible_kinds.push(kind);
                                    let (tile_action, id, star_id, inspect_id) = self.tile(
                                        ui,
                                        module,
                                        catalog,
                                        card.translate(content.min.to_vec2()),
                                        patch_version,
                                    );
                                    tile_ids.push((kind, id));
                                    other_ids.extend([star_id, inspect_id]);
                                    if tile_action.is_some() {
                                        action = tile_action;
                                    }
                                }
                            }
                        });
                });
            });
        let (inspection_rect, inspection_ids) = self.show_inspection(ctx, catalog, enabled);
        if !enabled {
            self.focused_last_frame = None;
            return None;
        }
        let outside_press = ctx.input(|input| {
            input.pointer.any_pressed()
                && input.pointer.interact_pos().is_some_and(|pos| {
                    !panel.response.rect.contains(pos)
                        && !inspection_rect.is_some_and(|rect| rect.contains(pos))
                })
        });
        let focused_id = ctx.memory(|memory| memory.focused());
        let focus = focused_id.and_then(|id| {
            if search_id == Some(id) {
                Some(FocusKind::Search)
            } else if tile_ids.iter().any(|(_, tile_id)| *tile_id == id) {
                Some(FocusKind::Tile)
            } else if inspection_ids.contains(&id) {
                Some(FocusKind::Inspection)
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
        // egui clears keyboard focus on Escape before widgets draw. Our own
        // open menu still owns that Escape, including its invisible sizing pass.
        let owns_keys = window_focused
            && !outside_press
            && (previous_focus.is_some()
                || focus.is_some()
                || zoom_popup_was_open
                || zoom_popup_active);
        if owns_keys {
            for (pressed, key) in [
                (keys.0, egui::Key::Enter),
                (keys.1, egui::Key::Space),
                (keys.2, egui::Key::Escape),
                (keys.7, egui::Key::I),
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
        if keys.2 && zoom_popup_was_open {
            if let Some(id) = zoom_id {
                egui::Popup::close_id(ctx, id.with("popup"));
                ctx.memory_mut(|memory| memory.request_focus(id));
            }
            self.focused_last_frame = Some(FocusKind::Other);
            return None;
        }
        if keys.2 && self.inspected.take().is_some() {
            self.request_focus();
            return None;
        }
        if keys.2 {
            *open = false;
            self.focused_last_frame = None;
            return None;
        }
        if keys.7 {
            if let Some(kind) = self.highlighted.clone() {
                self.inspect(&kind, catalog);
            }
        }
        let inspection_has_focus =
            focus == Some(FocusKind::Inspection) || previous_focus == Some(FocusKind::Inspection);
        let tile_has_focus =
            focus == Some(FocusKind::Tile) || previous_focus == Some(FocusKind::Tile);
        let other_has_focus = focus == Some(FocusKind::Other)
            || previous_focus == Some(FocusKind::Other)
            || zoom_popup_active
            || zoom_popup_was_open;
        if other_has_focus {
            for (pressed, key) in [
                (keys.3, egui::Key::ArrowLeft),
                (keys.4, egui::Key::ArrowRight),
                (keys.5, egui::Key::ArrowUp),
                (keys.6, egui::Key::ArrowDown),
            ] {
                if pressed {
                    consume_key(ctx, key);
                }
            }
        }
        let step = if inspection_has_focus || other_has_focus {
            0
        } else if keys.5 || (tile_has_focus && keys.3) {
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
            && !inspection_has_focus
            && !other_has_focus
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
            } else if inspection_ids.contains(&id) {
                Some(FocusKind::Inspection)
            } else if other_ids.contains(&id) {
                Some(FocusKind::Other)
            } else {
                None
            }
        });
        action
    }

    fn show_inspection(
        &mut self,
        ctx: &egui::Context,
        catalog: &Value,
        enabled: bool,
    ) -> (Option<Rect>, Vec<egui::Id>) {
        let Some(kind) = self.inspected.clone() else {
            return (None, Vec::new());
        };
        let Some(module) =
            modules(catalog).find(|module| module["kind"].as_str() == Some(kind.as_str()))
        else {
            self.inspected = None;
            return (None, Vec::new());
        };
        let preview = self
            .previews
            .entry(kind.clone())
            .or_insert_with(|| ModulePreview::new(module));
        if !preview.matches(module) {
            *preview = ModulePreview::new(module);
        }
        let viewport = ctx.content_rect();
        let mut ids = Vec::new();
        let mut close = false;
        let shown = egui::Window::new("Module inspection")
            .id(egui::Id::new("module-library-inspection"))
            .default_pos(viewport.min + vec2(22.0, 54.0))
            .default_width(600.0)
            .default_height((viewport.height() - 120.0).clamp(320.0, 620.0))
            .min_width(240.0)
            .max_width((viewport.width() - 32.0).max(240.0))
            .max_height((viewport.height() - 32.0).max(160.0))
            .constrain_to(viewport)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    // A dedicated focus target represents the display area;
                    // disabled panel controls never become keyboard targets.
                    let background = ui.interact(
                        ui.max_rect(),
                        ui.make_persistent_id("inspection-focus"),
                        Sense::focusable_noninteractive(),
                    );
                    ids.push(background.id);
                    ui.horizontal(|ui| {
                        ui.strong(
                            module["title"]
                                .as_str()
                                .unwrap_or_else(|| crate::rack::title(&kind)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let button = ui.button("Close inspection");
                            ids.push(button.id);
                            close = button.clicked();
                        });
                    });
                    ui.label(
                        egui::RichText::new(format!(
                            "Pinned · {} · Display only",
                            Category::of_kind(&kind).label()
                        ))
                        .size(11.0),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt(("inspection-details", &kind))
                        .max_height((viewport.height() - 168.0).max(80.0))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Scroll to see the full panel and details.")
                                    .small(),
                            );
                            // Keep the real hardware at its natural, readable scale.
                            // Wide panels scroll horizontally rather than shrinking.
                            egui::ScrollArea::both()
                                .id_salt(("inspection-panel", &kind))
                                .max_height(416.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    let size = preview.size();
                                    let (bounds, _) = ui.allocate_exact_size(size, Sense::hover());
                                    preview.show(ui, bounds, catalog);
                                });
                            ui.separator();
                            if let Some(description) = module["description"].as_str() {
                                ui.label(description);
                            }
                            for (field, label) in [("inputs", "Inputs"), ("outputs", "Outputs")] {
                                let ports = module[field]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>();
                                ui.label(format!(
                                    "{label}: {}",
                                    if ports.is_empty() {
                                        "None".to_owned()
                                    } else {
                                        ports.join(", ")
                                    }
                                ));
                            }
                            ui.label(format!(
                                "Engine: v{} or later",
                                module["min_patch_version"].as_u64().unwrap_or(1)
                            ));
                            let controls = visible_parameter_details(module);
                            ui.strong(format!("Controls ({})", controls.len()));
                            if controls.is_empty() {
                                ui.label("No adjustable controls.");
                            }
                            for (name, details) in controls {
                                ui.label(egui::RichText::new(name).strong());
                                ui.label(details);
                                ui.add_space(3.0);
                            }
                        });
                });
            });
        if enabled && !close {
            if let Some(shown) = shown.as_ref() {
                let pointer = ctx.input(|input| {
                    (input.pointer.any_pressed() || input.pointer.any_released())
                        .then(|| input.pointer.interact_pos())
                        .flatten()
                });
                if pointer.is_some_and(|pos| {
                    shown.response.rect.contains(pos)
                        && ctx.layer_id_at(pos) == Some(shown.response.layer_id)
                }) {
                    if let Some(id) = ids.first() {
                        ctx.memory_mut(|memory| memory.request_focus(*id));
                    }
                }
            }
        }
        if close {
            self.inspected = None;
            // Return focus only after an explicit library control closes it.
            self.request_focus();
        }
        (shown.map(|shown| shown.response.rect), ids)
    }

    fn tile(
        &mut self,
        ui: &mut egui::Ui,
        module: &Value,
        catalog: &Value,
        rect: Rect,
        patch_version: u32,
    ) -> (Option<LibraryAction>, egui::Id, egui::Id, egui::Id) {
        let kind = module["kind"].as_str().unwrap_or("");
        let title = module["title"]
            .as_str()
            .unwrap_or_else(|| crate::rack::title(kind));
        let supported = available(module, patch_version);
        let id = ui.make_persistent_id(("module-library-tile", kind));
        // Only the hardware/title region adds or drags. Footer controls have
        // disjoint hit rectangles and can never accidentally insert a module.
        let add_rect = Rect::from_min_max(rect.min, rect.max - vec2(0.0, 32.0));
        let response = ui.interact(add_rect, id, Sense::click_and_drag());
        if self.scroll_to_kind.as_deref() == Some(kind) {
            response.scroll_to_me(Some(egui::Align::Min));
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
        if ui.is_rect_visible(rect) {
            paint_tile(
                ui.painter(),
                rect,
                module,
                self.highlighted.as_deref() == Some(kind),
                response.hovered(),
                supported,
            );
            let natural = vec2(crate::rack::panel_width(kind), PANEL_HEIGHT) * self.preview_zoom;
            let preview_bounds = Rect::from_min_size(
                pos2(rect.center().x - natural.x * 0.5, rect.top() + 30.0),
                natural,
            );
            let preview = self
                .previews
                .entry(kind.to_owned())
                .or_insert_with(|| ModulePreview::new(module));
            if !preview.matches(module) {
                *preview = ModulePreview::new(module);
            }
            preview.show(ui, preview_bounds, catalog);
            if !supported {
                ui.painter()
                    .rect_filled(preview_bounds, 0, Color32::from_black_alpha(105));
                ui.painter().text(
                    preview_bounds.center_bottom() - vec2(0.0, 11.0),
                    Align2::CENTER_CENTER,
                    "ENGINE V2",
                    FontId::proportional(10.0),
                    PANEL_TEXT,
                );
            }
        }
        let star_rect =
            Rect::from_center_size(rect.right_bottom() + vec2(-16.0, -15.0), Vec2::splat(25.0));
        let star = ui.interact(star_rect, id.with("favorite"), Sense::click());
        if star.has_focus() && !ui.clip_rect().contains_rect(star.rect) {
            star.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
        }
        let favorite = self.favorites.contains(kind);
        star.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!(
                    "{} {title} {} favorites",
                    if favorite { "Remove" } else { "Add" },
                    if favorite { "from" } else { "to" }
                ),
            )
        });
        ui.painter().text(
            star_rect.center(),
            Align2::CENTER_CENTER,
            if favorite { "★" } else { "☆" },
            FontId::proportional(17.0),
            if favorite { AMBER } else { PANEL_MUTED },
        );
        let star = star.on_hover_text(if favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        });
        let inspect_rect =
            Rect::from_min_size(rect.left_bottom() + vec2(6.0, -28.0), vec2(62.0, 25.0));
        let inspect = ui.interact(inspect_rect, id.with("inspect"), Sense::click());
        if inspect.has_focus() && !ui.clip_rect().contains_rect(inspect.rect) {
            inspect.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
        }
        inspect.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                enabled,
                format!("Inspect {title} module"),
            )
        });
        let visuals = ui.style().interact(&inspect);
        ui.painter().rect(
            inspect_rect,
            3.0,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            inspect_rect.center(),
            Align2::CENTER_CENTER,
            "Inspect",
            FontId::proportional(11.0),
            visuals.text_color(),
        );
        let inspect = inspect.on_hover_text("Pin a larger panel and module details (Ctrl/Cmd+I)");
        let mut action = None;
        if enabled && inspect.clicked() {
            inspect.request_focus();
            self.inspect(kind, catalog);
        } else if enabled && star.clicked() {
            star.request_focus();
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
        response.on_hover_ui(|ui| {
            // Reuse the exact cached rack renderer. This inspection view is
            // disabled hardware, just like the tile, and never owns input.
            if let Some(preview) = self.previews.get_mut(kind) {
                let viewport = ui.ctx().content_rect();
                let size = inspection_preview_size(preview.size(), viewport.size());
                ui.set_width(size.x.max(260.0).min((viewport.width() - 32.0).max(1.0)));
                egui::ScrollArea::vertical()
                    .max_height((viewport.height() - 32.0).max(1.0))
                    .show(ui, |ui| {
                        ui.label(&tooltip);
                        ui.separator();
                        let (bounds, _) = ui.allocate_exact_size(size, Sense::hover());
                        preview.show(ui, bounds, catalog);
                    });
            }
        });
        (action, id, star.id, inspect.id)
    }
}

/// Flow uses the visible viewport width, never a wide prior content extent.
fn preview_flow(modules: &[&Value], zoom: f32, viewport_width: f32) -> (Vec<Rect>, Vec2) {
    let height = PANEL_HEIGHT * zoom + 62.0;
    let gap = 8.0;
    let mut cursor = Vec2::ZERO;
    let mut width = 0.0_f32;
    let mut cards = Vec::with_capacity(modules.len());
    for module in modules {
        let kind = module["kind"].as_str().unwrap_or("");
        let card_width = (crate::rack::panel_width(kind) * zoom + 16.0).max(108.0);
        if cursor.x > 0.0 && cursor.x + card_width > viewport_width {
            cursor.x = 0.0;
            cursor.y += height + gap;
        }
        let rect = Rect::from_min_size(cursor.to_pos2(), vec2(card_width, height));
        width = width.max(rect.right());
        cards.push(rect);
        cursor.x += card_width + gap;
    }
    (
        cards,
        vec2(
            width,
            if modules.is_empty() {
                0.0
            } else {
                cursor.y + height
            },
        ),
    )
}

/// Catalog presentation fields only; runtime/hidden parameters never enter details.
fn visible_parameter_details(module: &Value) -> Vec<(String, String)> {
    module["params"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, spec)| spec["hidden"] != true)
        .map(|(name, spec)| {
            let unit = spec["unit"].as_str().unwrap_or("");
            let mut parts = Vec::new();
            if let Some(default) = spec["default"].as_f64() {
                parts.push(format!("Default: {default} {unit}").trim().to_owned());
            }
            if let (Some(min), Some(max)) = (spec["min"].as_f64(), spec["max"].as_f64()) {
                parts.push(format!("Range: {min} to {max} {unit}").trim().to_owned());
            }
            if let Some(description) = spec["description"].as_str() {
                parts.push(description.to_owned());
            }
            (name.clone(), parts.join(" · "))
        })
        .collect()
}

fn inspection_preview_size(natural: Vec2, viewport: Vec2) -> Vec2 {
    let available = vec2(
        (viewport.x - 32.0).clamp(1.0, 760.0),
        (viewport.y - 200.0).clamp(1.0, 396.0),
    );
    natural
        * (available.x / natural.x)
            .min(available.y / natural.y)
            .min(1.0)
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
    _supported: bool,
) {
    // The card is browser chrome; only the shared rack renderer paints hardware.
    painter.rect_filled(
        rect.translate(vec2(0.0, 2.0)),
        5.0,
        Color32::from_black_alpha(55),
    );
    painter.rect_filled(
        rect,
        5.0,
        if hovered {
            Color32::from_rgb(48, 53, 51)
        } else {
            Color32::from_rgb(29, 33, 32)
        },
    );
    painter.rect_stroke(
        rect,
        5.0,
        Stroke::new(
            if highlighted { 1.5_f32 } else { 1.0 },
            if highlighted {
                AMBER
            } else {
                Color32::from_rgb(67, 72, 67)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let kind = module["kind"].as_str().unwrap_or("");
    let title = module["title"]
        .as_str()
        .unwrap_or_else(|| crate::rack::title(kind));
    let mut job = egui::text::LayoutJob::simple(
        title.to_owned(),
        crate::panel_skin::print_font(painter.ctx(), 14.0),
        PANEL_TEXT,
        rect.width() - 39.0,
    );
    job.wrap.max_rows = 1;
    painter.galley(
        rect.min + vec2(10.0, 9.0),
        painter.layout_job(job),
        PANEL_TEXT,
    );
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
    fn shift_register_search_excludes_hidden_capture_state_and_uses_musical_category() {
        let catalog = openpatch_core::catalog();
        let module = modules(&catalog)
            .find(|module| module["kind"] == "sh_asr")
            .unwrap();
        let library = ModuleLibrary::default();
        for hidden in ["value1", "value8", "rng_hi", "rng_lo"] {
            assert!(
                !library.matches(module, hidden),
                "hidden runtime field {hidden} is not a control"
            );
        }
        for visible in ["seed", "random", "range", "in8", "trig8", "out8"] {
            assert!(
                library.matches(module, visible),
                "visible parameter or port {visible} remains searchable"
            );
        }
        assert_eq!(Category::of_kind("sh_asr"), Category::Modulation);
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
        assert_eq!(Category::of_kind("stock_vcf"), Category::Filters);
        assert_eq!(Category::of_kind("stock_delay"), Category::Mixing);
        assert_eq!(Category::of_kind("poly_split"), Category::Routing);
        assert_eq!(Category::of_kind("pulses"), Category::Utilities);
        assert_eq!(Category::of_kind("push"), Category::Utilities);
        assert_eq!(Category::of_kind("poly_viz"), Category::Utilities);
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
    fn actual_preview_fields_and_buttons_keep_tile_click_drag_and_focus() {
        fn target(shapes: &[egui::epaint::ClippedShape], kind: &str) -> egui::Pos2 {
            fn find(shape: &egui::Shape, kind: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Vec(children) => {
                        children.iter().find_map(|shape| find(shape, kind))
                    }
                    egui::Shape::Text(text)
                        if match kind {
                            "seq3" => text.galley.text().ends_with(" BPM"),
                            "poly_constant" => text.galley.text().ends_with(" V"),
                            "push" => text.galley.text() == "HOLD",
                            "sh_asr" => text.galley.text() == "CLEAR",
                            "constant" => text.galley.text() == "0.000",
                            _ => false,
                        } =>
                    {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                }
            }
            shapes
                .iter()
                .find_map(|shape| find(&shape.shape, kind))
                .unwrap_or_else(|| panic!("missing actual {kind} field/button"))
        }
        fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }
        let all = openpatch_core::catalog();
        for kind in ["seq3", "poly_constant", "push", "sh_asr", "constant"] {
            let module = all["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|module| module["kind"] == kind)
                .unwrap();
            let catalog = json!({"modules": [module]});
            let ctx = egui::Context::default();
            let mut library = ModuleLibrary::default();
            let mut open = true;
            let (_, output) = frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
            let card_width = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(29, 33, 32) => {
                        Some(rect.rect.width())
                    }
                    _ => None,
                })
                .fold(0.0_f32, f32::max);
            assert_eq!(
                card_width,
                (crate::rack::panel_width(kind) * 0.5 + 16.0).max(108.0),
                "{kind} retains natural default panel scale"
            );
            let start = target(&output.shapes, kind);
            // Native compact fields and performance buttons are painted inside
            // the card at its chosen scale, but the card owns every gesture.
            frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                1.0,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
                true,
            );
            let (action, _) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                1.1,
                vec![pointer(start, false)],
                true,
            );
            assert_eq!(
                action,
                Some(LibraryAction::Add(kind.to_owned())),
                "click {kind}"
            );
            // Focus belongs to a tile, never its disabled controls.
            assert_eq!(
                library.focused_last_frame,
                Some(FocusKind::Tile),
                "focus {kind}"
            );
            assert!(!egui::Popup::is_any_open(&ctx));
            frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                2.0,
                vec![egui::Event::PointerMoved(start), pointer(start, true)],
                true,
            );
            let (action, _) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                2.1,
                vec![egui::Event::PointerMoved(start + vec2(40.0, 25.0))],
                true,
            );
            assert_eq!(action, None, "drag {kind}");
            assert_eq!(
                egui::DragAndDrop::payload::<LibraryDrag>(&ctx).map(|drag| drag.kind.clone()),
                Some(kind.to_owned())
            );
            frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                2.2,
                vec![pointer(start + vec2(40.0, 25.0), false)],
                true,
            );
        }
    }

    #[test]
    fn offscreen_cards_do_not_construct_or_render_previews() {
        let catalog = openpatch_core::catalog();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        frame(
            &egui::Context::default(),
            &mut library,
            &mut open,
            &catalog,
            0.0,
            vec![],
            true,
        );
        assert!(!library.previews.is_empty());
        assert!(library.previews.len() < catalog["modules"].as_array().unwrap().len());
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

    #[test]
    fn hover_inspection_reuses_actual_wide_preview_without_focus_or_library_actions() {
        fn bpm_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<(Rect, String)> {
            fn collect(shape: &egui::Shape, found: &mut Vec<(Rect, String)>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text().ends_with(" BPM") => {
                        found.push((
                            Rect::from_min_size(text.pos, text.galley.size()),
                            text.galley.text().to_owned(),
                        ));
                    }
                    egui::Shape::Vec(children) => {
                        for child in children {
                            collect(child, found);
                        }
                    }
                    _ => {}
                }
            }
            let mut found = Vec::new();
            for shape in shapes {
                collect(&shape.shape, &mut found);
            }
            found
        }
        let catalog = openpatch_core::catalog();
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|module| module["kind"] == "seq3")
            .unwrap();
        let original = module.clone();
        let ctx = egui::Context::default();
        ctx.style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let mut library = ModuleLibrary::default();
        library.set_preview_zoom(0.25);
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(940.0, 680.0));
        let mut render = |time: f64, hover: bool| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    time: Some(time),
                    focused: true,
                    events: if hover {
                        vec![egui::Event::PointerMoved(pos2(75.0, 90.0))]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let (cards, size) =
                            preview_flow(&[module], library.preview_zoom(), ui.available_width());
                        let (bounds, _) = ui.allocate_exact_size(size, Sense::hover());
                        let card = cards[0].translate(bounds.min.to_vec2());
                        let (action, _, _, _) = library.tile(ui, module, &catalog, card, 2);
                        assert!(action.is_none());
                    });
                },
            )
        };
        let initial = render(0.0, false);
        let tile_height = bpm_text(&initial.shapes)
            .iter()
            .map(|(rect, _)| rect.height())
            .fold(0.0_f32, f32::max);
        assert!(
            tile_height > 0.0,
            "actual tile contains the native tempo display"
        );
        render(1.0, true);
        render(2.0, true);
        let enlarged = render(3.0, true);
        let labels = bpm_text(&enlarged.shapes);
        fn has_metadata(shape: &egui::Shape, description: &str) -> bool {
            match shape {
                egui::Shape::Text(text) => {
                    text.galley.text().contains("Implementation:")
                        && text.galley.text().contains(description)
                }
                egui::Shape::Vec(children) => children
                    .iter()
                    .any(|shape| has_metadata(shape, description)),
                _ => false,
            }
        }
        assert!(enlarged
            .shapes
            .iter()
            .any(|shape| has_metadata(&shape.shape, module["description"].as_str().unwrap())));
        assert!(
            labels
                .iter()
                .any(|(rect, _)| rect.height() > tile_height * 2.0),
            "hover paints a larger actual tempo display"
        );
        assert!(labels.iter().all(|(rect, _)| viewport.contains_rect(*rect)));
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        assert!(!egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(&ctx));
        assert!(library.favorites().is_empty());
        assert!(library.highlighted.is_none());
        assert_eq!(library.previews.len(), 1);
        let preview = library.previews.get("seq3").unwrap();
        assert!(preview.matches(&original));
        let size = inspection_preview_size(preview.size(), viewport.size());
        assert!((size.x / size.y - preview.size().x / preview.size().y).abs() < 0.0001);
        assert!(size.x <= viewport.width() - 32.0 && size.y <= viewport.height() - 200.0);
        assert_eq!(module, &original);
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn explicit_inspect_footer_never_adds_drags_or_changes_favorites() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        let (_, output) = frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        let inspect = text_center(&output, "Inspect");
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            let (action, _) = frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                time,
                vec![
                    egui::Event::PointerMoved(inspect),
                    pointer(inspect, pressed),
                ],
                true,
            );
            assert_eq!(action, None);
            assert!(!egui::DragAndDrop::has_payload_of_type::<LibraryDrag>(&ctx));
        }
        assert_eq!(library.inspected.as_deref(), Some("analog_vco"));
        assert!(library.favorites().is_empty());
        assert!(!library.take_favorites_changed());
        assert_eq!(library.focused_last_frame, Some(FocusKind::Other));
        library.inspect("ladder_vcf", &catalog);
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            2.0,
            vec![key(egui::Key::Enter, true)],
            true,
        );
        assert_eq!(
            action, None,
            "Enter on Inspect must never activate the tile"
        );
    }

    #[test]
    fn inspection_pin_survives_search_filters_and_refreshes_replaced_catalog_metadata() {
        let ctx = egui::Context::default();
        let mut catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        assert!(library.inspect("analog_vco", &catalog));
        assert!(!library.inspect("missing", &catalog));
        library.search = "filter".into();
        library.category = Category::Filters;
        frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        assert_eq!(library.highlighted.as_deref(), Some("ladder_vcf"));
        assert_eq!(library.inspected.as_deref(), Some("analog_vco"));
        library.search.clear();
        catalog["modules"][0]["params"]["frequency"]["default"] = json!(440);
        frame(&ctx, &mut library, &mut open, &catalog, 1.0, vec![], true);
        assert!(library.previews["analog_vco"].matches(&catalog["modules"][0]));
        assert_eq!(library.inspected.as_deref(), Some("analog_vco"));
        catalog["modules"].as_array_mut().unwrap().remove(0);
        frame(&ctx, &mut library, &mut open, &catalog, 2.0, vec![], true);
        assert_eq!(library.inspected, None, "removed modules close safely");
        assert!(
            library.inspect("ladder_vcf", &catalog),
            "inspection can be reopened"
        );
    }

    #[test]
    fn escape_closes_inspection_before_library_and_search_remains_usable() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.request_focus();
        library.inspect("analog_vco", &catalog);
        frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            1.0,
            vec![key(egui::Key::Escape, true)],
            true,
        );
        assert_eq!(action, None);
        assert!(open);
        assert!(library.inspected.is_none());
        frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            1.1,
            vec![key(egui::Key::Escape, false)],
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
        assert_eq!(action, Some(LibraryAction::Add("analog_vco".into())));
        frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            2.1,
            vec![key(egui::Key::Enter, false)],
            true,
        );
        frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            3.0,
            vec![key(egui::Key::Escape, true)],
            true,
        );
        assert!(!open);
    }

    #[test]
    fn inspect_shortcut_requires_library_focus_and_consumes_only_its_own_shortcut() {
        let catalog = fixture();
        for focused in [false, true] {
            let ctx = egui::Context::default();
            let mut library = ModuleLibrary::default();
            let mut open = true;
            if focused {
                library.request_focus();
            }
            frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
            let modifiers = egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            let mut action = None;
            let mut global_sees_shortcut = false;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(940.0, 680.0))),
                    focused: true,
                    modifiers,
                    events: vec![egui::Event::Key {
                        key: egui::Key::I,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    action = library.show(ctx, &mut open, &catalog, 2, true);
                    global_sees_shortcut = ctx.input(|input| input.key_pressed(egui::Key::I));
                },
            );
            assert_eq!(action, None);
            assert_eq!(library.inspected.is_some(), focused);
            assert_eq!(global_sees_shortcut, !focused);
        }
    }

    #[test]
    fn inspection_details_exclude_hidden_fields_and_show_visible_ranges_units_and_ports() {
        let catalog = openpatch_core::catalog();
        let module = modules(&catalog)
            .find(|module| module["kind"] == "sh_asr")
            .unwrap();
        let details = visible_parameter_details(module);
        for hidden in ["value1", "value8", "rng_hi", "rng_lo"] {
            assert!(!details.iter().any(|(name, _)| name == hidden));
        }
        assert!(details.iter().any(|(name, text)| name == "seed"
            && text.contains("Default:")
            && text.contains("Range:")));
        let seq3 = modules(&catalog)
            .find(|module| module["kind"] == "seq3")
            .unwrap();
        assert!(visible_parameter_details(seq3)
            .iter()
            .any(|(name, text)| name == "tempo" && text.contains("BPM")));
        let original = catalog.clone();
        let mut library = ModuleLibrary::default();
        library.inspect("seq3", &catalog);
        let ctx = egui::Context::default();
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(940.0, 680.0));
        let mut render = || {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(viewport),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    let (rect, _) = library.show_inspection(ctx, &catalog, true);
                    assert!(
                        viewport.contains_rect(rect.unwrap()),
                        "small-screen inspection stays bounded"
                    );
                },
            )
        };
        render(); // egui measures a newly opened window before painting it.
        let output = render();
        fn tempo_rect(shape: &egui::Shape) -> Option<Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().ends_with(" BPM") => {
                    Some(Rect::from_min_size(text.pos, text.galley.size()))
                }
                egui::Shape::Vec(children) => children.iter().find_map(tempo_rect),
                _ => None,
            }
        }
        let tempo = output
            .shapes
            .iter()
            .find_map(|shape| tempo_rect(&shape.shape))
            .expect("inspection paints the actual native tempo field");
        assert!(viewport.contains_rect(tempo));
        assert!(
            tempo.height() > 10.0,
            "native controls stay readable at natural scale"
        );
        assert!(library.previews["seq3"].matches(seq3));
        assert_eq!(
            catalog, original,
            "inspection cannot mutate catalog/defaults"
        );
        assert!(
            ctx.memory(|memory| memory.focused()).is_none(),
            "opening a display does not take keyboard focus"
        );
    }

    #[test]
    fn inspection_focus_never_adds_and_outside_press_releases_keyboard_ownership() {
        let ctx = egui::Context::default();
        let catalog = fixture();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        library.inspect("analog_vco", &catalog);
        // A new window settles its geometry in the first frame.
        frame(&ctx, &mut library, &mut open, &catalog, 0.0, vec![], true);
        let (_, output) = frame(&ctx, &mut library, &mut open, &catalog, 0.1, vec![], true);
        let detail = text_center(&output, "Pinned · Oscillators · Display only");
        for (time, pressed) in [(1.0, true), (1.1, false)] {
            frame(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                time,
                vec![egui::Event::PointerMoved(detail), pointer(detail, pressed)],
                true,
            );
        }
        assert_eq!(library.focused_last_frame, Some(FocusKind::Inspection));
        let highlighted = library.highlighted.clone();
        let (action, _) = frame(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            2.0,
            vec![key(egui::Key::Enter, true), key(egui::Key::ArrowDown, true)],
            true,
        );
        assert_eq!(action, None);
        assert_eq!(library.highlighted, highlighted);
        let outside = pos2(4.0, 720.0);
        let mut global_sees_enter = false;
        let mut action = None;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1100.0, 750.0))),
                focused: true,
                time: Some(3.0),
                events: vec![
                    egui::Event::PointerMoved(outside),
                    pointer(outside, true),
                    key(egui::Key::Enter, true),
                ],
                ..Default::default()
            },
            |ctx| {
                action = library.show(ctx, &mut open, &catalog, 2, true);
                global_sees_enter = ctx.input(|input| input.key_pressed(egui::Key::Enter));
            },
        );
        assert_eq!(action, None);
        assert!(global_sees_enter);
        assert_eq!(library.focused_last_frame, None);
        assert!(
            library.inspected.is_some(),
            "outside clicks keep the optional pin open"
        );
    }
}

#[cfg(test)]
mod library_zoom_tests {
    use super::*;
    include!("library_zoom_tests.rs");
}
