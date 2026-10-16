//! One motion preference for egui's native popup fades.
//!
//! `MenuButton`, submenus, `ComboBox`, and context menus all use `Area`'s
//! quadratic-out opening fade. Configure its duration before constructing UI
//! instead of moving painted meshes away from their input rectangles. Keeping
//! egui in charge preserves popup placement, clipping, focus and dismissal.
use crate::construction_motion::MotionMode;
use eframe::egui;

const FULL_SECONDS: f32 = 0.160;
const REDUCED_SECONDS: f32 = 0.080;

pub(crate) fn configure(ctx: &egui::Context, mode: MotionMode, deterministic_capture: bool) {
    let seconds = if deterministic_capture {
        0.0
    } else {
        match mode {
            MotionMode::Full => FULL_SECONDS,
            MotionMode::Reduced => REDUCED_SECONDS,
            MotionMode::Off => 0.0,
        }
    };
    // This is egui's shared UI animation duration, including native Area fades.
    // It also keeps the framework's hover transitions consistent with the same
    // preference. Avoid cloning the shared style when nothing changed.
    if ctx.style().animation_time != seconds {
        ctx.style_mut(|style| style.animation_time = seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Menu,
        Combo,
    }

    struct Harness {
        ctx: egui::Context,
        mode: MotionMode,
        kind: Kind,
        trigger: Option<egui::Id>,
        opacity: Option<f32>,
        item: Option<egui::Rect>,
        layer: Option<egui::LayerId>,
        chosen: usize,
        background_clicks: usize,
    }

    impl Harness {
        fn new(kind: Kind, mode: MotionMode) -> Self {
            Self {
                ctx: egui::Context::default(),
                mode,
                kind,
                trigger: None,
                opacity: None,
                item: None,
                layer: None,
                chosen: 0,
                background_clicks: 0,
            }
        }

        fn frame(&mut self, time: f64, events: Vec<egui::Event>) {
            self.opacity = None;
            self.item = None;
            self.layer = None;
            let ctx = self.ctx.clone();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 360.0),
                    )),
                    time: Some(time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    configure(ctx, self.mode, false);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let kind = self.kind;
                        let mut content = |ui: &mut egui::Ui| {
                            self.opacity = Some(ui.opacity());
                            self.layer = Some(ui.layer_id());
                            let item = ui.button("Choose item");
                            self.item = Some(item.rect);
                            if item.clicked() {
                                self.chosen += 1;
                                ui.close();
                            }
                        };
                        let trigger = match kind {
                            Kind::Menu => ui.menu_button("File", &mut content).response,
                            Kind::Combo => {
                                egui::ComboBox::from_id_salt("motion-choice")
                                    .selected_text("Choice")
                                    .show_ui(ui, &mut content)
                                    .response
                            }
                        };
                        self.trigger = Some(trigger.id);
                        // Deliberately underneath the popup. A fade must never
                        // disable menu hit testing or allow the click through.
                        if ui
                            .put(
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 32.0),
                                    egui::pos2(500.0, 340.0),
                                ),
                                egui::Button::new("Background"),
                            )
                            .clicked()
                        {
                            self.background_clicks += 1;
                        }
                    });
                },
            );
        }

        fn open(&mut self, time: f64) {
            egui::Popup::open_id(&self.ctx, self.trigger.unwrap().with("popup"));
            // A new Area needs a sizing pass, as it does in the real app.
            self.frame(time, vec![]);
            self.frame(time + 0.01, vec![]);
            assert!(self.item.is_some(), "{:?} did not open", self.kind);
        }
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn native_menu_and_combo_fade_then_reopen_from_the_start() {
        for kind in [Kind::Menu, Kind::Combo] {
            let mut h = Harness::new(kind, MotionMode::Full);
            h.frame(0.0, vec![]);
            h.open(1.0);
            let opening = h.opacity.unwrap();
            assert!(opening > 0.0 && opening < 0.5, "{kind:?}: {opening}");
            let item = h.item.unwrap();
            let layer = h.layer.unwrap();
            assert!(h.ctx.layer_transform_to_global(layer).is_none());
            h.frame(1.08, vec![]);
            let middle = h.opacity.unwrap();
            assert!(middle > opening && middle < 1.0, "{kind:?}: {middle}");
            assert_eq!(h.item.unwrap(), item, "fade moved the hit rectangle");
            h.frame(1.20, vec![]);
            assert_eq!(h.opacity, Some(1.0));

            egui::Popup::close_all(&h.ctx);
            h.frame(1.21, vec![]);
            assert!(h.opacity.is_none());
            h.open(1.22);
            let reopened = h.opacity.unwrap();
            assert!(reopened > 0.0 && reopened < 0.5, "{kind:?}: {reopened}");
        }
    }

    #[test]
    fn faded_items_remain_immediately_clickable_and_block_the_background() {
        for kind in [Kind::Menu, Kind::Combo] {
            let mut h = Harness::new(kind, MotionMode::Full);
            h.frame(0.0, vec![]);
            h.open(1.0);
            let point = h.item.unwrap().center();
            h.frame(1.02, pointer(point, true));
            h.frame(1.03, pointer(point, false));
            assert_eq!(h.chosen, 1, "{kind:?} delayed the action during its fade");
            assert_eq!(h.background_clicks, 0, "{kind:?} let the click through");
            assert!(!egui::Popup::is_any_open(&h.ctx));
        }
    }

    #[test]
    fn reduced_and_off_modes_apply_to_actual_popups_and_can_change_live() {
        for kind in [Kind::Menu, Kind::Combo] {
            let mut h = Harness::new(kind, MotionMode::Reduced);
            h.frame(0.0, vec![]);
            h.open(1.0);
            assert!(h.opacity.unwrap() < 1.0);
            h.frame(1.09, vec![]);
            assert_eq!(h.opacity, Some(1.0));
            assert_eq!(h.ctx.style().animation_time, REDUCED_SECONDS);

            egui::Popup::close_all(&h.ctx);
            h.frame(1.10, vec![]);
            h.mode = MotionMode::Full;
            h.open(1.11);
            assert!(h.opacity.unwrap() < 1.0);
            h.mode = MotionMode::Off;
            h.frame(1.13, vec![]);
            assert_eq!(h.opacity, Some(1.0));
            assert_eq!(h.ctx.style().animation_time, 0.0);
        }
    }

    #[test]
    fn escape_and_outside_click_keep_native_dismissal_during_the_fade() {
        for kind in [Kind::Menu, Kind::Combo] {
            let mut h = Harness::new(kind, MotionMode::Full);
            h.frame(0.0, vec![]);
            h.open(1.0);
            h.frame(
                1.02,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert!(!egui::Popup::is_any_open(&h.ctx));
            h.frame(1.03, vec![]);
            h.open(1.04);
            let point = egui::pos2(450.0, 300.0);
            h.frame(1.06, pointer(point, true));
            h.frame(1.07, pointer(point, false));
            assert!(!egui::Popup::is_any_open(&h.ctx));
            assert_eq!(h.chosen, 0);
        }
    }

    #[test]
    fn captures_settle_immediately_and_regular_frames_restore_motion() {
        let ctx = egui::Context::default();
        configure(&ctx, MotionMode::Full, true);
        assert_eq!(ctx.style().animation_time, 0.0);
        configure(&ctx, MotionMode::Full, false);
        assert_eq!(ctx.style().animation_time, FULL_SECONDS);
    }
}
