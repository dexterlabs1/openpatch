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
