use super::*;

#[test]
fn gate_configured_pulse_follows_saved_duty_without_fabricating_playback() {
    for duty in [0.25, 0.75] {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{
            "id":"clock","kind":"gate","params":{"frequency":2,"duty":duty}
        }]}))
        .unwrap();
        patch.compile().unwrap();
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack::default();
        let mut previous = None;
        for time in [0.0, 13.0] {
            let (action, output) =
                frame_revision_state(&ctx, &mut rack, &patch, time, vec![], true, true, 9);
            assert!(action.is_none());
            let pulse = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Path(path)
                        if path.points.len() == 4
                            && path.stroke.color
                                == egui::epaint::ColorMode::Solid(Color32::from_rgb(
                                    229, 183, 103,
                                )) =>
                    {
                        Some(path.points.clone())
                    }
                    _ => None,
                })
                .expect("The configured pulse is drawn");
            let expected_edge = 24.0 + 26.0 + 98.0 * duty;
            assert!((pulse[1].x - expected_edge).abs() < 0.01);
            assert_eq!(pulse[1].x, pulse[2].x);
            assert!(pulse[0].y < pulse[3].y, "The configured period starts high");
            if let Some(previous) = previous {
                assert_eq!(
                    pulse, previous,
                    "UI time never pretends to be playback phase"
                );
            }
            previous = Some(pulse);
        }
        assert!(rack.module_lights.is_empty());
        assert!(rack.sequence_states.is_empty());
    }
}
