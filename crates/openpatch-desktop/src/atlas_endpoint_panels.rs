// Passive module arithmetic, not signal telemetry or host/device status.
// Core's scalar engines agree: Mixer = bounded(a+b), Output = a,
// AudioOut = a*0.1. Rack output selection is a separate shared UI state.
fn atlas_endpoint_kind(kind: &str) -> bool {
    matches!(kind, "mixer" | "output" | "audio_out")
}

fn paint_atlas_endpoint_configuration(
    painter: &egui::Painter,
    node: &Node,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let width = rect.width() / zoom;
    let center = width * 0.5;
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
    let ink = Color32::from_rgb(195, 200, 180);
    let quiet = Color32::from_rgb(133, 143, 123);
    let line = Color32::from_rgb(112, 125, 103);
    let amber = Color32::from_rgb(229, 183, 103);
    let arrow = |x1, y1, x2, y2| {
        painter.arrow(
            p(x1, y1),
            vec2(x2 - x1, y2 - y1) * zoom,
            Stroke::new(1.0 * zoom, line),
        );
    };
    let start = crate::lighting::next(painter);
    let draw = AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout { width },
        zoom,
    };
    // The screen is a printed topology/calibration guide. It never animates,
    // and its illumination must not imply measured input or output activity.
    draw.screen(
        Rect::from_min_max(p(12.0, 76.0), p(width - 12.0, 275.0)),
        lighting,
    );
    text(center, 91.0, "SIGNAL PATH", 9.0, ink);
    match node.kind.as_str() {
        "mixer" => {
            text(35.0, 123.0, "A", 12.0, ink);
            text(115.0, 123.0, "B", 12.0, ink);
            arrow(35.0, 137.0, 61.0, 157.0);
            arrow(115.0, 137.0, 89.0, 157.0);
            painter.circle_stroke(p(center, 169.0), 17.0 * zoom, Stroke::new(1.0 * zoom, line));
            text(center, 169.0, "+", 24.0, amber);
            arrow(center, 191.0, center, 211.0);
            text(center, 228.0, "OUT = A + B", 11.0, ink);
            text(center, 247.0, "UNITY SUM", 9.0, quiet);
            text(center, 264.0, "RAW INTERNAL UNITS", 8.0, quiet);
            // ±1,000,000 is Core's safety bound, not a ±1 audio limiter.
            text(center, 290.0, "SAFETY LIMIT ±1,000,000", 8.0, INK);
        }
        "output" => {
            text(center, 119.0, "SIGNAL", 10.0, ink);
            arrow(center, 132.0, center, 150.0);
            text(center, 169.0, "× 1", 23.0, amber);
            arrow(center, 188.0, center, 206.0);
            text(center, 224.0, "OUT = IN", 11.0, ink);
            text(center, 251.0, "UNITY ROUTE", 8.5, quiet);
            text(center, 290.0, "RAW INTERNAL UNITS", 8.0, INK);
        }
        "audio_out" => {
            text(center, 116.0, "VOLTS IN", 10.0, ink);
            arrow(center, 129.0, center, 142.0);
            text(center, 160.0, "÷ 10", 23.0, amber);
            arrow(center, 179.0, center, 191.0);
            text(center, 207.0, "AUDIO UNITS", 9.0, ink);
            text(center, 233.0, "+10 V → +1.0", 9.0, quiet);
            text(center, 253.0, "−10 V → −1.0", 9.0, quiet);
            text(center, 290.0, "10 V = FULL SCALE", 8.0, INK);
        }
        _ => {}
    }
    // Input centers remain y=332, output y=374. Keep the lower jack area
    // completely clear; routing to the host is determined by rack selection,
    // and the final exporter/device's policies are not this module's math.
    lighting.preserve(painter, start);
}
