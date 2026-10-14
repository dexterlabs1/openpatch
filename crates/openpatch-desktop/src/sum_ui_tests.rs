use super::*;

#[test]
fn sum_level_uses_shared_exact_entry_and_retains_revision() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({
        "version":2,"nodes":[{"id":"sum","kind":"poly_sum"}]
    }))
    .unwrap();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 7);
    // Rack origin (24,12), SUM's LEVEL center (75,145).
    right_click_parameter(&ctx, &mut rack, &patch, 7, pos2(99.0, 157.0), 1.0);
    let edit = rack.editing.as_mut().expect("SUM level exact-entry popup");
    assert_eq!(edit.parameter, "level");
    assert_eq!((edit.min, edit.max), (0.0, 1.0));
    edit.text = "0.375".into();
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.3,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
        true,
        7,
    )
    .0;
    assert!(
        matches!(action, Some(RackAction::Command("set_parameter", args))
        if args["id"] == "sum" && args["parameter"] == "level"
        && args["value"] == 0.375 && args["expected_revision"] == 7)
    );
}

#[test]
fn sum_unpublished_and_live_channel_displays_are_distinct() {
    fn labels(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    labels(shape, out);
                }
            }
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({
        "version":2,"nodes":[{"id":"sum","kind":"poly_sum"}]
    }))
    .unwrap();
    for channels in [None, Some(0), Some(16)] {
        rack.module_lights.clear();
        if let Some(channels) = channels {
            rack.module_lights.insert(
                "sum".into(),
                ModuleLightState {
                    count: 6,
                    input_channels: Some(channels),
                    ..ModuleLightState::default()
                },
            );
        }
        let output = frame_revision_state(&ctx, &mut rack, &patch, 1.0, vec![], true, true, 7).1;
        let mut text = Vec::new();
        for shape in &output.shapes {
            labels(&shape.shape, &mut text);
        }
        assert!(text.contains(&channels.map_or_else(|| "— / 16".into(), |c| format!("{c} / 16"))));
        assert_eq!(
            text.iter().any(|s| s == "METER UNAVAILABLE"),
            channels.is_none()
        );
        for db in ["0", "-3", "-6", "-12", "-24", "-36"] {
            assert!(text.iter().any(|s| s == db));
        }
    }
    assert_eq!(panel_width("poly_sum"), 150.0);
}

#[test]
fn sum_ports_patch_independently_below_the_meter() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"sum","kind":"poly_sum"},
        {"id":"source","kind":"constant"},
        {"id":"sink","kind":"audio_out"}
    ]}))
    .unwrap();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    let input = fixture_jack(&rack, &patch, "sum", "signal", true);
    let output = fixture_jack(&rack, &patch, "sum", "out", false);
    assert_eq!(input, pos2(69.0, 379.0));
    assert_eq!(output, pos2(129.0, 379.0));
    // Verify painted meter text remains above both socket hit regions.
    let painted = frame_revision_state(&ctx, &mut rack, &patch, 0.1, vec![], true, true, 7).1;
    fn assert_meter_clear(shape: &egui::Shape, input: Pos2, output: Pos2) {
        match shape {
            egui::Shape::Text(text)
                if text.galley.text() == "METER UNAVAILABLE"
                    || text.galley.text() == "OUT · 10 V = 0 dB"
                    || ["0", "-3", "-6", "-12", "-24", "-36"].contains(&text.galley.text()) =>
            {
                let bounds = Rect::from_min_size(text.pos, text.galley.size());
                for socket in [input, output] {
                    assert!(
                        !bounds.intersects(Rect::from_center_size(socket, vec2(36.0, 36.0))),
                        "meter text {} overlaps socket",
                        text.galley.text()
                    );
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    assert_meter_clear(shape, input, output);
                }
            }
            _ => {}
        }
    }
    for shape in &painted.shapes {
        assert_meter_clear(&shape.shape, input, output);
    }
    assert!(input.distance(output) > 2.0 * 18.0);
    let source = fixture_jack(&rack, &patch, "source", "out", false);
    let sink = fixture_jack(&rack, &patch, "sink", "signal", true);
    let action = drag(&ctx, &mut rack, &patch, 1.0, source, input);
    assert!(
        matches!(action, Some(RackAction::Command("connect", ref args))
        if args["from"] == "source" && args["to"] == "sum" && args["input"] == "signal")
    );
    let action = drag(&ctx, &mut rack, &patch, 3.0, output, sink);
    assert!(
        matches!(action, Some(RackAction::Command("connect", ref args))
        if args["from"] == "sum" && args["to"] == "sink" && args["output"] == "out")
    );
    assert!(rack.editing.is_none());
}
