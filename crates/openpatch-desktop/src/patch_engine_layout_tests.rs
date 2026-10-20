use super::*;

fn layout(size: egui::Vec2, zoom: f32) -> (egui::Rect, egui::Rect, egui::Rect) {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let mut panel = Panel { open: true, ..Panel::default() };
    let ctx = egui::Context::default();
    ctx.set_zoom_factor(zoom);
    let mut screen = egui::Rect::NOTHING;
    for frame in 0..3 {
        let output = ctx.run(
            egui::RawInput {
                // The raw screen is in physical pixels at one pixel per native point.
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(frame as f64 * 0.1),
                ..Default::default()
            },
            |ctx| {
                screen = ctx.content_rect();
                panel.show(ctx, &session);
            },
        );
        drop(output);
    }
    let rect = |key| ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(key))).unwrap();
    (screen, rect("patch-engine-words-rect"), rect("patch-engine-options-rect"))
}

#[test]
fn build_options_never_cover_the_word_lists_at_any_interface_size() {
    // Steam Deck 1280×800 from one point per pixel up to oversized Game Mode scales, plus a desktop window.
    for (size, zoom) in [
        (egui::vec2(1280.0, 800.0), 1.0),
        (egui::vec2(1280.0, 800.0), 1.5),
        (egui::vec2(1280.0, 800.0), 2.25),
        (egui::vec2(1280.0, 800.0), 3.0),
        (egui::vec2(1600.0, 1000.0), 1.0),
    ] {
        let (screen, words, options) = layout(size, zoom);
        assert!(words.bottom() <= options.top() + 0.5, "{size:?} × {zoom}: words {words:?} run under options {options:?}");
        assert!(options.bottom() <= screen.bottom() + 0.5 && options.top() >= screen.top(), "{size:?} × {zoom}: options {options:?} off screen {screen:?}");
        if zoom <= 1.5 {
            assert!(words.height() >= 150.0, "{size:?} × {zoom}: word lists squeezed to {words:?}");
        }
    }
}

#[test]
fn controller_d_pad_chooses_rows_and_words_and_b_closes() {
    use crate::gamepad::{Input, Mapper};
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let mut panel = Panel { open: true, ..Panel::default() };
    let ctx = egui::Context::default();
    let mut mapper = Mapper::default();
    let mut time = 0.0;
    let mut press = |panel: &mut Panel, kind: u16, code: u16, value: i32| {
        for value in [value, 0] {
            let mut events = vec![];
            mapper.feed(Input { kind, code, value }, time, &mut events);
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
                    time: Some(time),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ctx| {
                    panel.show(ctx, &session);
                },
            );
            time += 0.05;
        }
    };
    let (key, abs) = (0x01, 0x03);
    press(&mut panel, abs, 0x11, 1); // hat down: Genre → Mood
    assert_eq!(panel.cursor, 1);
    press(&mut panel, abs, 0x10, 1); // hat right: first mood
    assert_eq!(panel.mood, Some(0));
    press(&mut panel, abs, 0x10, 1);
    assert_eq!(panel.mood, Some(1));
    press(&mut panel, key, 0x131, 1); // B
    assert!(!panel.open);
}
