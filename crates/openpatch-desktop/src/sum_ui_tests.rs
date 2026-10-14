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
