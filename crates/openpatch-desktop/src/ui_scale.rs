//! Interface size: a user-chosen scale on top of an automatic base.
//!
//! The base is 100% of the system scale, except on handheld gaming screens
//! (Steam Deck, gamescope Game Mode), whose reported scale draws the chrome
//! too large for a 1280×800 panel; there the base is one point per pixel,
//! matching Desktop Mode.
use eframe::egui;

/// Choices offered in Settings, relative to the automatic size.
pub(crate) const STEPS: [f32; 8] = [0.75, 0.85, 1.0, 1.15, 1.3, 1.5, 1.75, 2.0];

/// The persisted choice, clamped to the offered range.
pub(crate) fn from_settings(settings: &serde_json::Value) -> f32 {
    settings["ui_scale"].as_f64().map_or(1.0, |scale| {
        (scale as f32).clamp(STEPS[0], STEPS[STEPS.len() - 1])
    })
}

/// Steam Deck or a gamescope session. `OPENPATCH_HANDHELD=1|0` overrides detection.
pub(crate) fn handheld() -> bool {
    let var = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    match std::env::var("OPENPATCH_HANDHELD").as_deref() {
        Ok("1") => return true,
        Ok("0") => return false,
        _ => {}
    }
    if !cfg!(target_os = "linux") {
        return false;
    }
    let dmi = |field: &str| {
        std::fs::read_to_string(format!("/sys/devices/virtual/dmi/id/{field}"))
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    };
    std::env::var("SteamDeck").as_deref() == Ok("1")
        || var("GAMESCOPE_WAYLAND_DISPLAY")
        || (dmi("board_vendor") == "Valve"
            && matches!(dmi("product_name").as_str(), "Jupiter" | "Galileo"))
}

/// The smallest interface, in points, that lays out without overlap (the desktop minimum window).
pub(crate) const MIN_POINTS: egui::Vec2 = egui::vec2(940.0, 640.0);

/// The egui zoom factor for a user scale, given the system's points-per-pixel and the
/// window size in system points. Enlarging stops where the interface would no longer fit.
pub(crate) fn zoom_factor(
    scale: f32,
    native_pixels_per_point: Option<f32>,
    handheld: bool,
    window: egui::Vec2,
) -> f32 {
    let base = match native_pixels_per_point {
        Some(native) if handheld && native > 1.0 => 1.0 / native,
        _ => 1.0,
    };
    let fit = (window.x / MIN_POINTS.x).min(window.y / MIN_POINTS.y);
    if scale > 1.0 && window.min_elem() > 0.0 {
        (base * scale).min(fit.max(base))
    } else {
        base * scale
    }
}

/// Applies the scale; returns true when the interface changed size this frame.
pub(crate) fn apply(ctx: &egui::Context, scale: f32, handheld: bool) -> bool {
    // UI size belongs to Settings; Command +/- zoom the rack instead.
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    let window = ctx.content_rect().size() * ctx.zoom_factor();
    let target = zoom_factor(scale, ctx.native_pixels_per_point(), handheld, window);
    let changed = (ctx.zoom_factor() - target).abs() > 1e-4;
    if changed {
        ctx.set_zoom_factor(target);
    }
    changed
}

pub(crate) fn label(scale: f32) -> String {
    let percent = (scale * 100.0).round();
    if percent == 100.0 {
        "100% (Auto)".into()
    } else {
        format!("{percent}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handheld_base_is_one_point_per_pixel_and_desktop_keeps_the_system_scale() {
        let big = egui::vec2(4000.0, 3000.0);
        assert_eq!(zoom_factor(1.0, Some(2.25), true, big), 1.0 / 2.25);
        assert_eq!(zoom_factor(1.5, Some(2.0), true, big), 0.75);
        assert_eq!(zoom_factor(1.0, Some(1.0), true, big), 1.0);
        assert_eq!(zoom_factor(1.0, Some(2.0), false, big), 1.0);
        assert_eq!(zoom_factor(0.75, None, true, big), 0.75);
    }

    #[test]
    fn enlarging_stops_where_the_interface_would_no_longer_fit() {
        // Steam Deck Game Mode: 1280×800 pixels reported at 2.25 pixels per point.
        let deck = egui::vec2(1280.0, 800.0) / 2.25;
        let zoom = zoom_factor(2.0, Some(2.25), true, deck);
        assert!((deck.y / zoom - MIN_POINTS.y).abs() < 0.01, "{zoom}");
        assert_eq!(zoom_factor(1.0, Some(2.25), true, deck), 1.0 / 2.25);
        // Never smaller than Auto, and shrinking is always allowed.
        assert_eq!(zoom_factor(1.5, None, false, egui::vec2(800.0, 500.0)), 1.0);
        assert_eq!(
            zoom_factor(0.75, None, false, egui::vec2(800.0, 500.0)),
            0.75
        );
    }

    #[test]
    fn saved_scale_is_clamped_and_defaults_to_auto() {
        assert_eq!(from_settings(&serde_json::json!({})), 1.0);
        assert_eq!(from_settings(&serde_json::json!({"ui_scale": 1.3})), 1.3);
        assert_eq!(from_settings(&serde_json::json!({"ui_scale": 9.0})), 2.0);
        assert_eq!(from_settings(&serde_json::json!({"ui_scale": 0.1})), 0.75);
        assert_eq!(label(1.0), "100% (Auto)");
        assert_eq!(label(1.15), "115%");
    }
}
