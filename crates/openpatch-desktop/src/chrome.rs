//! Shared native-vector toolbar controls for the Studio Rack chrome.
use eframe::egui::{self, vec2, Color32, Rect, Response, Sense, Stroke, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    Undo,
    Redo,
    Save,
    Play,
    Pause,
    Library,
    Scope,
    Visuals,
    Settings,
    Minus,
    Plus,
}

/// Native click/focus semantics, an accessible name, and a common 30-point
/// target. Callers own every resulting action and selected state.
pub(crate) fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    enabled: bool,
) -> Response {
    let response = ui.add_enabled(enabled, |ui: &mut Ui| {
        let (_, response) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), selected, label)
        });
        response
    });
    if ui.is_rect_visible(response.rect) {
        let active = response.enabled();
        let green = selected && matches!(icon, Icon::Play | Icon::Pause);
        let amber = Color32::from_rgb(208, 155, 80);
        let accent = if green {
            Color32::from_rgb(135, 191, 146)
        } else {
            amber
        };
        let (fill, border, ink) = if !active {
            (
                Color32::from_rgb(33, 36, 36),
                Color32::from_rgb(48, 52, 50),
                Color32::from_rgb(88, 94, 89),
            )
        } else if response.is_pointer_button_down_on() {
            (Color32::from_rgb(80, 64, 43), accent, accent)
        } else if selected {
            (
                if green {
                    Color32::from_rgb(38, 57, 43)
                } else {
                    Color32::from_rgb(62, 53, 40)
                },
                accent,
                accent,
            )
        } else if response.hovered() || response.has_focus() {
            (
                Color32::from_rgb(51, 56, 54),
                amber,
                Color32::from_rgb(225, 218, 199),
            )
        } else {
            (
                Color32::from_rgb(40, 44, 43),
                Color32::from_rgb(65, 70, 66),
                Color32::from_rgb(194, 201, 190),
            )
        };
        ui.painter().rect_filled(response.rect, 5.0, fill);
        ui.painter().rect_stroke(
            response.rect,
            5.0,
            Stroke::new(
                if active && response.has_focus() {
                    2.0_f32
                } else {
                    1.0_f32
                },
                border,
            ),
            egui::StrokeKind::Inside,
        );
        paint_icon(ui.painter(), response.rect, icon, ink);
    }
    response.on_hover_text(label).on_disabled_hover_text(label)
}

fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, ink: Color32) {
    let point = |x, y| rect.center() + vec2(x - 10.0, y - 10.0);
    let stroke = Stroke::new(1.6_f32, ink);
    let path = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|&(x, y)| point(x, y)).collect(),
            stroke,
        ));
    };
    let outline = |x1, y1, x2, y2| {
        painter.rect_stroke(
            Rect::from_min_max(point(x1, y1), point(x2, y2)),
            0.5,
            stroke,
            egui::StrokeKind::Inside,
        );
    };
    match icon {
        Icon::Undo | Icon::Redo => {
            let direction = |x| if icon == Icon::Redo { 20.0 - x } else { x };
            path(&[
                (direction(5.0), 7.0),
                (direction(12.0), 7.0),
                (direction(16.0), 10.0),
                (direction(16.0), 14.0),
                (direction(13.0), 17.0),
                (direction(8.0), 17.0),
            ]);
            path(&[
                (direction(9.0), 3.0),
                (direction(5.0), 7.0),
                (direction(9.0), 11.0),
            ]);
        }
        Icon::Save => {
            path(&[
                (3.0, 3.0),
                (14.0, 3.0),
                (17.0, 6.0),
                (17.0, 17.0),
                (3.0, 17.0),
                (3.0, 3.0),
            ]);
            outline(6.0, 3.0, 12.0, 8.0);
            outline(6.0, 12.0, 14.0, 17.0);
        }
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![point(6.0, 3.0), point(17.0, 10.0), point(6.0, 17.0)],
                ink,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            painter.rect_filled(
                Rect::from_min_max(point(5.0, 4.0), point(8.0, 16.0)),
                0.5,
                ink,
            );
            painter.rect_filled(
                Rect::from_min_max(point(12.0, 4.0), point(15.0, 16.0)),
                0.5,
                ink,
            );
        }
        Icon::Library => {
            outline(3.0, 4.0, 7.0, 17.0);
            outline(9.0, 4.0, 13.0, 17.0);
            path(&[
                (14.0, 5.0),
                (17.0, 4.0),
                (20.0, 16.0),
                (17.0, 17.0),
                (14.0, 5.0),
            ]);
        }
        Icon::Scope => {
            outline(2.0, 3.0, 18.0, 17.0);
            path(&[
                (3.0, 11.0),
                (6.0, 11.0),
                (8.0, 6.0),
                (10.0, 15.0),
                (12.0, 9.0),
                (17.0, 9.0),
            ]);
        }
        Icon::Visuals => {
            path(&[
                (10.0, 3.0),
                (12.0, 8.0),
                (17.0, 10.0),
                (12.0, 12.0),
                (10.0, 17.0),
                (8.0, 12.0),
                (3.0, 10.0),
                (8.0, 8.0),
                (10.0, 3.0),
            ]);
            path(&[(16.0, 1.0), (16.0, 5.0)]);
            path(&[(14.0, 3.0), (18.0, 3.0)]);
        }
        Icon::Settings => {
            let mut teeth = Vec::with_capacity(33);
            for index in 0..=32 {
                let angle = index as f32 * std::f32::consts::TAU / 32.0;
                let radius = if index % 4 < 2 { 8.0 } else { 6.2 };
                teeth.push(point(
                    10.0 + radius * angle.cos(),
                    10.0 + radius * angle.sin(),
                ));
            }
            painter.add(egui::Shape::line(teeth, stroke));
            painter.circle_stroke(rect.center(), 2.8, stroke);
        }
        Icon::Minus | Icon::Plus => {
            path(&[(4.0, 10.0), (16.0, 10.0)]);
            if icon == Icon::Plus {
                path(&[(10.0, 4.0), (10.0, 16.0)]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        parent_enabled: bool,
    ) -> (Vec<Response>, egui::FullOutput) {
        let mut responses = Vec::new();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 100.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_enabled_ui(parent_enabled, |ui| {
                        ui.horizontal(|ui| {
                            responses.push(icon_button(
                                ui,
                                Icon::Undo,
                                "Undo last edit",
                                false,
                                true,
                            ));
                            responses.push(icon_button(
                                ui,
                                Icon::Save,
                                "Save project",
                                false,
                                false,
                            ));
                            responses.push(icon_button(ui, Icon::Play, "Play audio", true, true));
                        });
                    });
                });
            },
        );
        (responses, output)
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn keyboard_activation_reports_accessible_button_name_and_selected_state() {
        let ctx = egui::Context::default();
        let (buttons, _) = frame(&ctx, vec![], true);
        for button in &buttons {
            assert_eq!(button.rect.size(), vec2(30.0, 30.0));
        }
        ctx.memory_mut(|memory| memory.request_focus(buttons[2].id));
        let (buttons, output) = frame(&ctx, vec![key(egui::Key::Enter)], true);
        assert!(buttons[2].clicked());
        assert!(!buttons[0].clicked());
        let info = output
            .platform_output
            .events
            .iter()
            .find_map(|event| {
                if let egui::output::OutputEvent::Clicked(info) = event {
                    Some(info)
                } else {
                    None
                }
            })
            .expect("keyboard click should expose widget information");
        assert_eq!(info.typ, egui::WidgetType::Button);
        assert_eq!(info.label.as_deref(), Some("Play audio"));
        assert_eq!(info.selected, Some(true));
        assert!(info.enabled);
    }

    #[test]
    fn disabled_button_and_disabled_parent_reject_pointer_and_keyboard_activation() {
        for parent_enabled in [true, false] {
            let ctx = egui::Context::default();
            let (buttons, _) = frame(&ctx, vec![], parent_enabled);
            let index = if parent_enabled { 1 } else { 0 };
            assert!(!buttons[index].enabled());
            let target = buttons[index].rect.center();
            ctx.memory_mut(|memory| memory.request_focus(buttons[index].id));
            let (buttons, _) = frame(&ctx, vec![key(egui::Key::Space)], parent_enabled);
            assert!(!buttons[index].clicked());
            for pressed in [true, false] {
                let (buttons, _) = frame(
                    &ctx,
                    vec![
                        egui::Event::PointerMoved(target),
                        egui::Event::PointerButton {
                            pos: target,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    parent_enabled,
                );
                assert!(!buttons[index].clicked());
            }
        }
    }
}
