// Source-backed functional routing, never a simulation of live sample history.
fn atlas_capture_routing_kind(kind: &str) -> bool {
    matches!(
        kind,
        "sample_hold" | "mult" | "sh_asr" | "switch_1to4" | "switch_4to1"
    )
}
fn paint_capture_control_print(painter: &egui::Painter, rect: Rect, zoom: f32) {
    for (y, text, size, color) in [
        (73.0, "INITIAL ON LOAD", 9.0, INK),
        (86.0, "−16 TO +16 V", 7.0, MUTED),
    ] {
        painter.text(
            rect.min + vec2(75.0, y) * zoom,
            Align2::CENTER_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    }
}
fn paint_capture_routing_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let text = |x, y, value: &str, size, color| {
        painter.text(
            p(x, y),
            Align2::CENTER_CENTER,
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment(
            [p(a.0, a.1), p(b.0, b.1)],
            Stroke::new(0.9 * zoom, Color32::from_rgb(128, 123, 100)),
        );
    };
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    };
    if node.kind == "mult" {
        // The complete input frame fans out without normalization or arithmetic.
        line((150.0, 119.0), (150.0, 142.0));
        line((60.0, 142.0), (240.0, 142.0));
        line((150.0, 130.0), (276.0, 130.0));
        line((276.0, 130.0), (276.0, 315.0));
        line((60.0, 315.0), (276.0, 315.0));
        for x in [60.0, 120.0, 180.0, 240.0] {
            line((x, 142.0), (x, 150.0));
            line((x, 315.0), (x, 324.0));
        }
        let start = crate::lighting::next(painter);
        draw.screen(
            Rect::from_min_max(p(34.0, 223.0), p(266.0, 300.0)),
            lighting,
        );
        text(
            150.0,
            239.0,
            "ONE INPUT · EIGHT COPIES",
            10.0,
            Color32::from_rgb(195, 200, 180),
        );
        text(
            150.0,
            263.0,
            "OUT 1…8 = IN",
            14.0,
            Color32::from_rgb(229, 183, 103),
        );
        text(
            150.0,
            286.0,
            "0–16 CHANNELS · NO ARITHMETIC",
            8.0,
            Color32::from_rgb(157, 169, 146),
        );
        lighting.preserve(painter, start);
    } else {
        let start = crate::lighting::next(painter);
        draw.screen(
            Rect::from_min_max(p(15.0, 215.0), p(135.0, 291.0)),
            lighting,
        );
        text(
            75.0,
            229.0,
            "SIGNAL → HOLD → OUT",
            8.0,
            Color32::from_rgb(229, 183, 103),
        );
        text(
            75.0,
            249.0,
            "CAPTURE AT TRIG ↑ 1 V",
            8.0,
            Color32::from_rgb(195, 200, 180),
        );
        text(
            75.0,
            266.0,
            "REARM ≤ 0.1 V",
            8.0,
            Color32::from_rgb(195, 200, 180),
        );
        text(
            75.0,
            282.0,
            "CONFIGURED PATH · NO DROOP",
            6.5,
            Color32::from_rgb(157, 169, 146),
        );
        lighting.preserve(painter, start);
    }
}
