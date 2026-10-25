// Stock Unity shares one face, toggles and sockets in the rack and browser:
// two six-input mixers stacked top and bottom, each with a SUM/AVERAGE
// toggle, five level lights and MIX/INV outputs, plus a MERGE toggle between
// them. Only engine telemetry lights the level lamps.
const STOCK_UNITY_SECTION: f32 = 184.0;
const STOCK_UNITY_COLUMNS: [f32; 3] = [38.0, 105.0, 172.0];
fn stock_unity_port(input: bool, index: usize) -> Pos2 {
    if input {
        let section = (index / 6) as f32 * STOCK_UNITY_SECTION;
        let jack = index % 6;
        pos2(
            STOCK_UNITY_COLUMNS[jack % 3],
            70.0 + (jack / 3) as f32 * 44.0 + section,
        )
    } else {
        // MIX 1, INV 1, MIX 2, INV 2.
        let section = (index / 2) as f32 * STOCK_UNITY_SECTION;
        pos2(if index.is_multiple_of(2) { 71.0 } else { 139.0 }, 186.0 + section)
    }
}
fn stock_unity_port_label(port: &str) -> &str {
    match port {
        "ch1_1" | "ch2_1" => "IN 1",
        "ch1_2" | "ch2_2" => "IN 2",
        "ch1_3" | "ch2_3" => "IN 3",
        "ch1_4" | "ch2_4" => "IN 4",
        "ch1_5" | "ch2_5" => "IN 5",
        "ch1_6" | "ch2_6" => "IN 6",
        "mix1" | "mix2" => "MIX",
        "inv1" | "inv2" => "INV",
        other => port_label(other),
    }
}
/// Toggle face rectangle and label for each Unity control.
fn stock_unity_toggle(name: &str) -> Option<(Rect, &'static str)> {
    match name {
        "average1" => Some((
            Rect::from_min_size(pos2(122.0, 130.0), vec2(72.0, 22.0)),
            "AVERAGE",
        )),
        "average2" => Some((
            Rect::from_min_size(pos2(122.0, 130.0 + STOCK_UNITY_SECTION), vec2(72.0, 22.0)),
            "AVERAGE",
        )),
        "merge" => Some((
            Rect::from_min_size(pos2(55.0, 206.0), vec2(100.0, 16.0)),
            "MERGE 1 + 2",
        )),
        _ => None,
    }
}
fn stock_unity_toggle_hover(name: &str) -> &'static str {
    if name == "merge" {
        "Join both sections into one twelve-input mix. Both sets of outputs then carry it, and each section's AVERAGE still applies to its own outputs."
    } else {
        "Divide the section's sum by the number of patched inputs, so the output stays at the level of a single input. A patched input counts even at 0 V; unpatched inputs never count."
    }
}
/// Lamp centres of one section, quietest first: -24, -18, -12, -6 dB, clip.
fn stock_unity_lamp(section: usize, lamp: usize) -> Pos2 {
    pos2(
        28.0 + lamp as f32 * 19.0,
        140.0 + section as f32 * STOCK_UNITY_SECTION,
    )
}
fn paint_stock_unity_configuration(
    ui: &mut egui::Ui,
    rect: Rect,
    node: &Node,
    zoom: f32,
    lights: Option<&ModuleLightState>,
    lighting: &crate::lighting::RoomLighting,
) {
    let painter = ui.painter().clone();
    let p = |point: Pos2| rect.min + point.to_vec2() * zoom;
    let lamps = lights.filter(|state| state.count >= 10);
    for section in 0..2 {
        for lamp in 0..5 {
            // Module light order is clip, -6, -12, -18, -24 dB per section.
            let light = section * 5 + (4 - lamp);
            let strength = lamps.map_or(0.0, |state| state.brightness[light][0].clamp(0.0, 1.0));
            let color = match lamp {
                4 => Color32::from_rgb(232, 92, 74),
                3 => Color32::from_rgb(236, 196, 84),
                _ => Color32::from_rgb(110, 204, 122),
            };
            let center = p(stock_unity_lamp(section, lamp));
            painter.circle_filled(center, 3.4 * zoom, Color32::from_gray(78));
            lighting.circle(
                &painter,
                center,
                2.6 * zoom,
                color.gamma_multiply(0.12 + strength * 0.88),
                strength,
                0.12,
            );
            painter.text(
                center + vec2(0.0, 11.0 * zoom),
                Align2::CENTER_CENTER,
                ["-24", "-18", "-12", "-6", "0"][lamp],
                crate::panel_skin::print_font(ui.ctx(), 7.0 * zoom),
                MUTED,
            );
        }
        let meter = Rect::from_center_size(
            p(pos2(66.0, 144.0 + section as f32 * STOCK_UNITY_SECTION)),
            vec2(96.0, 26.0) * zoom,
        );
        ui.interact(
            meter,
            ui.id().with(("stock-unity-meter", section, &node.id)),
            Sense::hover(),
        )
        .on_hover_text("Level of this section's mix, in decibels relative to 10 V. Lights rise with the peak at once and fall back over a fraction of a second; 0 lights at 10 V and above.");
    }
}
