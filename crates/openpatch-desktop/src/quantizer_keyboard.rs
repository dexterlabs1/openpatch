fn scale_keyboard(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    origin: Pos2,
    width: f32,
    zoom: f32,
    id: egui::Id,
    mask: u16,
) -> Option<u16> {
    let key_width = (width - 40.0) / 7.0;
    let mut changed = None;
    painter.text(
        origin + vec2(width * 0.5, 86.0) * zoom,
        Align2::CENTER_CENTER,
        "ENABLED NOTES",
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
    for (white_index, (note, label)) in [
        (0, "C"),
        (2, "D"),
        (4, "E"),
        (5, "F"),
        (7, "G"),
        (9, "A"),
        (11, "B"),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::from_min_size(
            origin + vec2(20.0 + white_index as f32 * key_width, 101.0) * zoom,
            vec2(key_width - 1.0, 76.0) * zoom,
        );
        let enabled = mask & (1 << note) != 0;
        let response = ui.interact(rect, id.with(note), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                enabled,
                format!("Scale note {label}"),
            )
        });
        painter.rect_filled(
            rect,
            1.0,
            if enabled {
                Color32::from_rgb(225, 203, 152)
            } else {
                Color32::from_rgb(240, 235, 219)
            },
        );
        painter.rect_stroke(
            rect,
            1.0,
            Stroke::new(0.8 * zoom, Color32::from_rgb(112, 107, 88)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center_bottom() - vec2(0.0, 10.0) * zoom,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            INK,
        );
        if response.clicked() {
            changed = Some(mask ^ (1 << note));
        }
        response.on_hover_text(format!("Toggle {label} in the quantizer scale"));
    }
    for (boundary, note, label) in [
        (1, 1, "C#"),
        (2, 3, "D#"),
        (4, 6, "F#"),
        (5, 8, "G#"),
        (6, 10, "A#"),
    ] {
        let rect = Rect::from_min_size(
            origin + vec2(20.0 + boundary as f32 * key_width - key_width * 0.3, 101.0) * zoom,
            vec2(key_width * 0.6, 46.0) * zoom,
        );
        let enabled = mask & (1 << note) != 0;
        let response = ui.interact(rect, id.with(note), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                true,
                enabled,
                format!("Scale note {label}"),
            )
        });
        painter.rect_filled(
            rect,
            1.0,
            if enabled {
                Color32::from_rgb(137, 103, 54)
            } else {
                Color32::from_rgb(39, 44, 41)
            },
        );
        painter.text(
            rect.center_bottom() - vec2(0.0, 10.0) * zoom,
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 7.0 * zoom),
            Color32::from_rgb(240, 235, 219),
        );
        if response.clicked() {
            changed = Some(mask ^ (1 << note));
        }
        response.on_hover_text(format!("Toggle {label} in the quantizer scale"));
    }
    for (x, label, value) in [
        (width * 0.5 - 28.0, "All", 4095),
        (width * 0.5 + 28.0, "None", 0),
    ] {
        let rect = Rect::from_center_size(origin + vec2(x, 194.0) * zoom, vec2(44.0, 18.0) * zoom);
        let response = ui.interact(rect, id.with(label), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("{label} scale notes"),
            )
        });
        painter.rect_filled(
            rect,
            2.0,
            if response.hovered() {
                Color32::from_rgb(205, 196, 170)
            } else {
                Color32::from_rgb(220, 213, 190)
            },
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
            INK,
        );
        if response.clicked() {
            changed = Some(value);
        }
    }
    changed
}
