// Configuration explains each production Core's different pitch law. These
// guides deliberately contain no synthetic notes, staircase or signal trace.
fn atlas_quantizer_control(kind: &str) -> (Pos2, f32) {
    if kind == "stock_quantizer" {
        (pos2(70.0, 307.0), 23.0)
    } else {
        (pos2(60.0, 263.0), 25.0)
    }
}

fn atlas_quantizer_value_center(kind: &str) -> Pos2 {
    if kind == "stock_quantizer" {
        pos2(70.0, 344.0)
    } else {
        pos2(60.0, 310.0)
    }
}

fn paint_atlas_quantizer_print(painter: &egui::Painter, rect: Rect, kind: &str, zoom: f32) {
    let stock = kind == "stock_quantizer";
    let (center, _) = atlas_quantizer_control(kind);
    for (y, text, size, color) in [
        (if stock { 258.0 } else { 216.0 }, "OFFSET", 9.0, INK),
        (
            if stock { 270.0 } else { 229.0 },
            if stock {
                "−12 TO +12 st"
            } else {
                "−4 TO +4 V"
            },
            7.0,
            MUTED,
        ),
    ] {
        painter.text(
            rect.min + vec2(center.x, y) * zoom,
            Align2::CENTER_CENTER,
            text,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    }
}

fn paint_atlas_quantizer_configuration(
    painter: &egui::Painter,
    kind: &str,
    mask: u16,
    rect: Rect,
    zoom: f32,
    lighting: &crate::lighting::RoomLighting,
) {
    let stock = kind == "stock_quantizer";
    let (left, top, right, bottom) = if stock {
        (119.0, 262.0, 282.0, 341.0)
    } else {
        (106.0, 216.0, 224.0, 303.0)
    };
    let p = |x, y| rect.min + vec2(x, y) * zoom;
    let bounds = Rect::from_min_max(p(left, top), p(right, bottom));
    let center = (left + right) * 0.5;
    let text = |y, value: &str, size, color| {
        painter.text(
            p(center, y),
            Align2::CENTER_CENTER,
            value,
            crate::panel_skin::print_font(painter.ctx(), size * zoom),
            color,
        );
    };
    // Like the other Atlas configuration wells, this is an illuminated printed
    // explanation; it is not a measured display and must not imply activity.
    let start = crate::lighting::next(painter);
    AtlasVoicePaint {
        painter,
        origin: rect.min,
        layout: AtlasVoiceLayout {
            width: rect.width() / zoom,
        },
        zoom,
    }
    .screen(bounds, lighting);
    let ink = Color32::from_rgb(203, 207, 185);
    let amber = Color32::from_rgb(229, 183, 103);
    let quiet = Color32::from_rgb(166, 177, 152);
    text(top + 11.0, "CONFIGURED PITCH PATH", 7.0, quiet);
    let bank = if mask == 0 {
        "EMPTY BANK".to_owned()
    } else {
        format!(
            "{} {} ENABLED",
            mask.count_ones(),
            if mask.count_ones() == 1 {
                "NOTE"
            } else {
                "NOTES"
            }
        )
    };
    text(top + 25.0, &bank, 8.0, amber);
    if stock {
        text(top + 39.0, "PITCH + OFFSET → 24 BINS / V", 7.0, ink);
        text(top + 52.0, "NEAREST INTEGER NOTE · TIES ↓", 7.0, ink);
        text(
            top + 66.0,
            if mask == 0 {
                "EMPTY → CHROMATIC"
            } else {
                "OUTPUT: ENABLED NOTE · 1 V/OCT"
            },
            7.0,
            amber,
        );
    } else {
        text(top + 39.0, "PITCH + OFFSET + CV", 7.0, ink);
        text(top + 53.0, "NEAREST NOTE · TIES ↑", 7.0, ink);
        text(
            top + 67.0,
            if mask == 0 {
                "EMPTY → PASS ADJUSTED"
            } else {
                "OUT: ENABLED NOTE"
            },
            7.0,
            amber,
        );
        text(top + 81.0, "1 V/OCT · MONO", 7.0, quiet);
    }
    lighting.preserve(painter, start);
}
