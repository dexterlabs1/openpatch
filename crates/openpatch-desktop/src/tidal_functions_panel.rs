// The configured relationships are printed guides; only engine telemetry lights
// the four output indicators. Rack and browser share these controls and ports.
fn tidal_control(name: &str) -> (Pos2, f32) {
    match name {
        "frequency" => (pos2(70.0, 145.0), 30.0),
        "slope" => (pos2(210.0, 145.0), 28.0),
        "shape" => (pos2(350.0, 145.0), 28.0),
        "smoothness" => (pos2(140.0, 248.0), 25.0),
        "shift" => (pos2(280.0, 248.0), 25.0),
        _ => unreachable!("Tidal Functions continuous control"),
    }
}
fn tidal_value_center(name: &str) -> Pos2 {
    let (center, _) = tidal_control(name);
    pos2(center.x, if center.y < 200.0 { 188.0 } else { 287.0 })
}
fn tidal_selector(name: &str) -> Option<(Rect, &'static str, &'static [&'static str])> {
    let (x, width, label, choices): (_, _, _, &[&str]) = match name {
        "ramp_mode" => (18.0, 72.0, "RAMP", &["AD", "LOOP", "AR"]),
        "output_mode" => (
            105.0,
            132.0,
            "OUTPUTS",
            &["GATES", "AMPLITUDE", "SLOPE / PHASE", "FREQUENCY"],
        ),
        "range" => (252.0, 78.0, "RANGE", &["CONTROL", "AUDIO"]),
        "clock_ratio" => (
            345.0,
            57.0,
            "CLOCK ×",
            &["1/16", "1/8", "1/4", "1/2", "1/1", "2", "4", "8", "16"],
        ),
        _ => return None,
    };
    Some((
        Rect::from_min_size(pos2(x, 66.0), vec2(width, 24.0)),
        label,
        choices,
    ))
}
fn tidal_port(input: bool, index: usize) -> Pos2 {
    if input {
        pos2(48.0 + index as f32 * 54.0, 338.0)
    } else {
        pos2(70.0 + index as f32 * 90.0, 378.0)
    }
}
fn tidal_port_label(port: &str) -> &str {
    match port {
        "slope_cv" => "SLOPE",
        "shape_cv" => "SHAPE",
        "smoothness_cv" => "SMOOTH",
        "shift_cv" => "SHIFT",
        "out1" => "1",
        "out2" => "2",
        "out3" => "3",
        "out4" => "4",
        other => port_label(other),
    }
}
fn paint_tidal_control_print(painter: &egui::Painter, rect: Rect, name: &str, zoom: f32) {
    let (center, _) = tidal_control(name);
    painter.text(
        rect.min + vec2(center.x, if center.y < 200.0 { 98.0 } else { 207.0 }) * zoom,
        Align2::CENTER_CENTER,
        name.to_uppercase(),
        crate::panel_skin::print_font(painter.ctx(), 10.0 * zoom),
        INK,
    );
}
fn tidal_relationship(node: &Node) -> &'static str {
    match node
        .params
        .get("output_mode")
        .copied()
        .unwrap_or(1.0)
        .round() as u8
    {
        0 => "1 SHAPED · 2 RAW · 3 END ATTACK · 4 END RELEASE",
        2 => match node.params.get("ramp_mode").copied().unwrap_or(1.0).round() as u8 {
            1 => "SHIFT SPACES FOUR RELATED PHASES",
            _ => "SHIFT SPREADS FOUR RELATED SLOPES",
        },
        3 => "SHIFT SELECTS FOUR RELATED FREQUENCIES",
        _ => "SHIFT SCANS FOUR OVERLAPPING LEVELS",
    }
}
fn paint_tidal_configuration(ui: &mut egui::Ui, rect: Rect, node: &Node, zoom: f32) {
    let guide = Rect::from_min_size(
        rect.min + vec2(18.0, 295.0) * zoom,
        vec2(384.0, 16.0) * zoom,
    );
    ui.painter().text(
        guide.center(),
        Align2::CENTER_CENTER,
        tidal_relationship(node),
        crate::panel_skin::print_font(ui.ctx(), 9.0 * zoom),
        INK,
    );
    ui.interact(guide, ui.id().with(("tidal-relationships", &node.id)), Sense::hover())
        .on_hover_text("Configured output relationships. Slope sets rise/fall balance; Shape bends the curve; Smoothness filters below center and folds above center. Shift changes the relationship selected by OUTPUTS. LOOP produces bipolar source volts; AD/AR produce unipolar functions. END ATTACK/RELEASE gates are 8 V. Output indicators show actual delayed engine voltages. Pitch is 1 V/oct; the other four CV inputs add voltage/5. A patched trigger takes priority over the extracted clock ramp. The clock multiplier applies only when clock is patched.");
}
