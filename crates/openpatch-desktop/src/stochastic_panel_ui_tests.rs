use super::*;
use crate::construction_motion::MotionMode;
use std::time::Instant;

#[test]
fn stochastic_mcp_cues_cover_rotaries_choices_and_full_integer_seeds() {
    for (kind, changes) in [
        (
            "white_noise",
            vec![("level", 1.0, 2.0), ("seed", 1.0, 16777215.0)],
        ),
        (
            "colored_noise",
            vec![("level", 1.0, 1.5), ("seed", 1.0, 16777215.0)],
        ),
        (
            "random",
            vec![
                ("rate", 1.0, 2.0),
                ("probability", 1.0, 0.5),
                ("spread", 1.0, 0.75),
                ("shape", 1.0, 0.25),
                ("rate_cv", 0.0, 0.5),
                ("probability_cv", 0.0, -0.5),
                ("spread_cv", 0.0, 0.5),
                ("shape_cv", 0.0, -0.5),
                ("unipolar", 0.0, 1.0),
                ("seed", 1.0, 16777215.0),
            ],
        ),
    ] {
        for zoom in [0.42, 1.0, 1.4] {
            let before: Patch =
                serde_json::from_value(json!({"version":2,"nodes":[{"id":"m","kind":kind}]}))
                    .unwrap();
            let mut after = before.clone();
            for (name, _, value) in &changes {
                after.nodes[0].params.insert((*name).into(), *value);
            }
            let ctx = egui::Context::default();
            let mut rack = Rack {
                zoom,
                ..Default::default()
            };
            frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 4);
            let receipt = json!({"plan_id":7,"completed_steps":1,"last_revision":5,"current_revision":5,
                "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":7,"step_index":0,
                    "before_revision":4,"revision":5,"label":"Set stochastic controls","new_modules":[],"new_cables":[],
                    "parameter_changes":changes.iter().map(|(name,old,new)|json!({"id":"m","kind":kind,"parameter":name,"old_effective":old,"new_effective":new})).collect::<Vec<_>>()}});
            let now = Instant::now();
            rack.observe_construction_transition_at(
                &before,
                &after,
                4,
                5,
                &receipt,
                MotionMode::Full,
                now,
            );
            rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(160)));
            assert!(
                frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 5)
                    .0
                    .is_none()
            );
            assert_eq!(
                rack.construction_anchors.len(),
                changes.len(),
                "all controls at {kind}/{zoom}"
            );
            for (name, _, _) in &changes {
                let anchor = &rack.construction_anchors[&("m".into(), (*name).into())];
                assert!(!anchor.old_text.is_empty() && !anchor.new_text.is_empty());
                assert!(anchor.rect.is_finite() && anchor.rect.is_positive());
                if matches!(*name, "seed" | "unipolar") {
                    assert!(anchor.marker.is_none());
                } else {
                    assert!(matches!(
                        anchor.marker,
                        Some(crate::construction_motion::Marker::Rotary { .. })
                    ));
                }
            }
            assert!(
                rack.module_lights.is_empty(),
                "an edit cue cannot fabricate signal telemetry"
            );
        }
    }
}

#[test]
fn random_seed_badge_opens_guarded_full_precision_entry() {
    let ctx = egui::Context::default();
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"random","kind":"random"}]}))
            .unwrap();
    let mut rack = Rack::default();
    frame_revision_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true, 11);
    let at = random_seed_badge_rect().center() + vec2(24.0, 12.0);
    for (t, pressed) in [(1.0, true), (1.1, false)] {
        assert!(frame_revision_state(
            &ctx,
            &mut rack,
            &patch,
            t,
            vec![egui::Event::PointerMoved(at), button(at, pressed)],
            true,
            true,
            11
        )
        .0
        .is_none());
    }
    let edit = rack
        .editing
        .as_mut()
        .expect("seed badge must open exact entry");
    assert_eq!(edit.parameter, "seed");
    assert_eq!(edit.revision, 11);
    assert_eq!(edit.step, Some(1.0));
    edit.text = "16777215".into();
    let action = frame_revision_state(
        &ctx,
        &mut rack,
        &patch,
        1.2,
        vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        true,
        true,
        11,
    )
    .0;
    assert!(
        matches!(action,Some(RackAction::Command("set_parameter",args)) if args["parameter"]=="seed" && args["value"]==16777215.0 && args["expected_revision"]==11)
    );
}
