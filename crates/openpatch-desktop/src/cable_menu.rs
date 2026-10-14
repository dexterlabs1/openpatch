//! Cable operations share one menu at either end of a patch lead.
use super::*;

#[derive(Clone)]
pub(super) struct CableMenu {
    origin: CableEnd,
    cables: Vec<Cable>,
    revision: u64,
}

const COLOR_NAMES: [&str; 6] = ["Coral", "Amber", "Teal", "Blue", "Violet", "Copper"];

fn rgb(color: Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

/// Closing every ancestor is essential: an open parent would cancel the draft
/// on the next frame, and must not consume the drag's eventual release.
fn menu_input_valid(ui: &egui::Ui) -> bool {
    ui.is_enabled()
        && ui.input(|i| {
            i.focused
                && !i.key_pressed(egui::Key::Escape)
                && !i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::WindowFocused(false) | egui::Event::PointerGone
                    )
                })
        })
}

fn pickup_response(ui: &mut egui::Ui, response: egui::Response) -> Option<bool> {
    let id = response.id.with("unmodified-pickup-press");
    let press = ui.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers,
            } => Some((*pos, *modifiers)),
            _ => None,
        })
    });
    if let Some((pos, modifiers)) = press {
        let valid = response.interact_rect.contains(pos)
            && modifiers == egui::Modifiers::NONE
            && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
            && menu_input_valid(ui);
        ui.data_mut(|d| d.insert_temp(id, valid));
    }
    let valid = menu_input_valid(ui) && ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let activated =
        valid && (response.clicked() || response.drag_started_by(PointerButton::Primary));
    if ui.input(|i| i.pointer.button_released(PointerButton::Primary)) || !menu_input_valid(ui) {
        ui.data_mut(|d| d.remove::<bool>(id));
    }
    if activated {
        let dragging = response.drag_started_by(PointerButton::Primary);
        egui::Popup::close_all(ui.ctx());
        Some(dragging)
    } else {
        None
    }
}

fn pickup_button(ui: &mut egui::Ui, label: &str) -> Option<bool> {
    let response = ui.add(egui::Button::new(label).sense(Sense::click_and_drag()));
    pickup_response(ui, response)
}

impl Rack {
    pub(super) fn open_cable_menu(&mut self, origin: CableEnd, patch: &Patch) {
        self.pending = None;
        self.cable_press = None;
        self.dragging_cable = false;
        let cables = ordered_endpoint_cables(patch, &origin)
            .into_iter()
            .cloned()
            .collect();
        self.cable_menu = Some(CableMenu {
            origin,
            cables,
            revision: self.observed_revision,
        });
    }

    fn menu_pickup(
        &mut self,
        menu: &CableMenu,
        operation: CableOperation,
        cables: &[Cable],
        color: Option<[u8; 3]>,
        dragging: bool,
    ) {
        self.pending = Some(PendingCable {
            origin: menu.origin.clone(),
            operation,
            shift_at_press: false,
            original: cables.first().cloned(),
            companions: cables.iter().skip(1).cloned().collect(),
            color,
            revision: menu.revision,
            stack_requested: false,
        });
        self.cable_press = None;
        self.dragging_cable = dragging;
    }

    pub(super) fn port_cable_menu(
        &mut self,
        ui: &mut egui::Ui,
        origin: &CableEnd,
        patch: &Patch,
        action: &mut Option<RackAction>,
    ) {
        if self.cable_menu.as_ref().is_none_or(|m| m.origin != *origin) {
            self.open_cable_menu(origin.clone(), patch);
        }
        let menu = self.cable_menu.as_ref().unwrap().clone();
        if !self.shortcuts_enabled || !menu_input_valid(ui) {
            ui.disable();
        }
        let (node, port) = match origin {
            CableEnd::Input { node, port } | CableEnd::Output { node, port } => (node, port),
        };
        ui.strong(format!("{node}.{port} · {} cables", menu.cables.len()));
        if menu.revision != self.observed_revision {
            ui.label("Patch changed. Reopen this menu to edit its cables.");
            if ui.button("Close").clicked() {
                egui::Popup::close_all(ui.ctx());
            }
            return;
        }
        ui.separator();
        let top = menu.cables.last();
        ui.add_enabled_ui(top.is_some(), |ui| {
            if ui.button("Delete top cable").clicked() {
                if let Some(top) = top {
                    *action = menu_delete(&menu, std::slice::from_ref(top));
                }
                egui::Popup::close_all(ui.ctx());
            }
            if let Some(dragging) = pickup_button(ui, "Clone top cable") {
                if let Some(top) = top {
                    self.menu_pickup(
                        &menu,
                        CableOperation::Clone,
                        std::slice::from_ref(top),
                        None,
                        dragging,
                    );
                }
            }
        });
        let can_create = patch.version >= 2
            || !matches!(origin, CableEnd::Input { .. })
            || menu.cables.is_empty();
        ui.add_enabled_ui(can_create, |ui| {
            if let Some(dragging) = pickup_button(ui, "Create new cable") {
                self.menu_pickup(&menu, CableOperation::Create, &[], None, dragging);
            }
            ui.menu_button("New cable colour", |ui| {
                for (color, name) in CABLES.iter().zip(COLOR_NAMES) {
                    let response = ui.add(
                        egui::Button::new(egui::RichText::new(format!("● {name}")).color(*color))
                            .sense(Sense::click_and_drag()),
                    );
                    if let Some(dragging) = pickup_response(ui, response) {
                        self.menu_pickup(
                            &menu,
                            CableOperation::Create,
                            &[],
                            Some(rgb(*color)),
                            dragging,
                        );
                        egui::Popup::close_all(ui.ctx());
                    }
                }
            });
        });
        if !can_create {
            ui.label("Another cable on this input requires engine v2.");
        }
        if menu.cables.len() > 1 {
            if let Some(dragging) = pickup_button(ui, "Move all cables") {
                self.menu_pickup(&menu, CableOperation::Move, &menu.cables, None, dragging);
            }
            if ui.button("Delete all cables").clicked() {
                *action = menu_delete(&menu, &menu.cables);
                egui::Popup::close_all(ui.ctx());
            }
        }
        if !menu.cables.is_empty() {
            ui.separator();
            ui.label("Individual cables");
        }
        egui::ScrollArea::vertical()
            .id_salt("cable-list")
            .max_height(260.0)
            .show(ui, |ui| {
                for cable in menu.cables.iter().rev() {
                    let other = match origin {
                        CableEnd::Input { .. } => format!("{}.{}", cable.from, cable.output),
                        CableEnd::Output { .. } => format!("{}.{}", cable.to, cable.input),
                    };
                    let label = format!("{} · {other}", cable.id.as_deref().unwrap_or("legacy"));
                    ui.menu_button(
                        egui::RichText::new(label).color(cable_instance_color(cable)),
                        |ui| {
                            if let Some(dragging) = pickup_button(ui, "Move this cable") {
                                self.menu_pickup(
                                    &menu,
                                    CableOperation::Move,
                                    std::slice::from_ref(cable),
                                    None,
                                    dragging,
                                );
                            }
                            if let Some(dragging) = pickup_button(ui, "Clone this cable") {
                                self.menu_pickup(
                                    &menu,
                                    CableOperation::Clone,
                                    std::slice::from_ref(cable),
                                    None,
                                    dragging,
                                );
                            }
                            if ui.button("Delete this cable").clicked() {
                                *action = menu_delete(&menu, std::slice::from_ref(cable));
                                egui::Popup::close_all(ui.ctx());
                            }
                            ui.separator();
                            ui.label("Cable colour");
                            let effective = rgb(cable_instance_color(cable));
                            for (color, name) in CABLES.iter().zip(COLOR_NAMES) {
                                if ui
                                    .add_enabled(
                                        rgb(*color) != effective,
                                        egui::Button::new(
                                            egui::RichText::new(format!("● {name}")).color(*color),
                                        ),
                                    )
                                    .clicked()
                                {
                                    *action = recolor(&menu, cable, rgb(*color));
                                    egui::Popup::close_all(ui.ctx());
                                }
                            }
                            ui.menu_button("Custom RGB…", |ui| {
                                let id = ui.id().with(("cable-rgb", menu.revision, &cable.id));
                                let mut color = ui
                                    .data_mut(|d| *d.get_temp_mut_or_insert_with(id, || effective));
                                ui.color_edit_button_srgb(&mut color);
                                ui.horizontal(|ui| {
                                    for (name, channel) in
                                        ["R", "G", "B"].into_iter().zip(&mut color)
                                    {
                                        ui.label(name);
                                        ui.add(egui::DragValue::new(channel).range(0..=255));
                                    }
                                });
                                ui.data_mut(|d| d.insert_temp(id, color));
                                if ui
                                    .add_enabled(
                                        color != effective,
                                        egui::Button::new("Apply colour"),
                                    )
                                    .clicked()
                                {
                                    *action = recolor(&menu, cable, color);
                                    egui::Popup::close_all(ui.ctx());
                                }
                            });
                        },
                    );
                }
            });
    }
}

fn recolor(menu: &CableMenu, cable: &Cable, color: [u8; 3]) -> Option<RackAction> {
    Some(RackAction::Command(
        "set_cable_color",
        json!({"cable_id":cable.id.as_ref()?,"color":color,"expected_revision":menu.revision}),
    ))
}

fn menu_delete(menu: &CableMenu, cables: &[Cable]) -> Option<RackAction> {
    if cables.len() == 1 {
        let cable = &cables[0];
        let mut args = cable.id.as_ref().map_or_else(
            || json!({"to":cable.to,"input":cable.input}),
            |id| json!({"cable_id":id}),
        );
        args["expected_revision"] = json!(menu.revision);
        Some(RackAction::Command("disconnect", args))
    } else {
        let ids = cables
            .iter()
            .map(|c| c.id.clone())
            .collect::<Option<Vec<_>>>()?;
        Some(RackAction::Command(
            "disconnect_cables",
            json!({"cable_ids":ids,"expected_revision":menu.revision}),
        ))
    }
}
