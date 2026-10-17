use super::*;
#[test]
fn detailed_effect_mcp_cues_follow_all_twenty_four_knobs() {
    use crate::construction_motion::MotionMode;
    let catalog = openpatch_core::catalog();
    for kind in ["stereo_delay", "stereo_reverb", "stock_delay"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let mut before: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"fx","kind":kind}]})).unwrap();
        let names: Vec<_> = module["params"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let mut after = before.clone();
        let changes: Vec<_> = names.iter().map(|name| {
            let spec = &module["params"][name];
            let min = spec["min"].as_f64().unwrap() as f32;
            let max = spec["max"].as_f64().unwrap() as f32;
            let old = min+(max-min)*0.25;
            let new = min+(max-min)*0.75;
            before.nodes[0].params.insert(name.clone(),old);
            after.nodes[0].params.insert(name.clone(),new);
            json!({"id":"fx","kind":kind,"parameter":name,"old_effective":old,"new_effective":new})
        }).collect();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = std::time::Instant::now();
        let receipt = json!({"plan_id":23,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":23,"step_index":0,"before_revision":9,"revision":10,"label":"Shape ambience","new_modules":[],"new_cables":[],"parameter_changes":changes}});
        rack.observe_construction_transition_at(
            &before,
            &after,
            9,
            10,
            &receipt,
            MotionMode::Full,
            now,
        );
        rack.set_construction_clock_override(Some(now + std::time::Duration::from_millis(150)));
        assert!(
            frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10)
                .0
                .is_none()
        );
        for name in names {
            let anchor = &rack.construction_anchors[&("fx".into(), name.clone())];
            let Some(crate::construction_motion::Marker::Rotary {
                center,
                radius,
                old,
                new,
            }) = &anchor.marker
            else {
                panic!("{kind}.{name}: rotary cue missing")
            };
            assert!(*radius > 10.0);
            assert!(
                (0.0..=1.0).contains(old) && (0.0..=1.0).contains(new) && old < new,
                "{kind}.{name}"
            );
            assert!(anchor.rect.contains(*center));
            let expected = if kind == "stock_delay" {
                let i = STOCK_DELAY_PARAMS.iter().position(|p| *p == name).unwrap();
                pos2(
                    46.0 + 86.0 * (i % 4) as f32,
                    if i < 4 { 120.0 } else { 220.0 },
                )
            } else {
                atlas_studio_effect_control(kind, &name).0
            };
            assert!(
                center.distance(expected + vec2(24.0, 12.0)) < 0.001,
                "{kind}.{name}"
            );
            assert_ne!(anchor.old_text, anchor.new_text);
        }
    }
}
#[test]
fn space_configured_rt60_follows_actual_session_edits_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session
        .replace_patch(
            serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"fx","kind":"stereo_reverb"}]}),
            )
            .unwrap(),
            None,
        )
        .unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    for (time, expected) in [
        (0.0, "RT60 3.68 s"),
        (0.1, "RT60 8.30 s"),
        (0.2, "RT60 3.68 s"),
    ] {
        if time == 0.1 {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"fx","parameter":"decay","value":1.0}),
                )
                .unwrap();
        }
        if time == 0.2 {
            session.execute("undo", json!({})).unwrap();
        }
        let (_, out) = frame_revision_state(
            &ctx,
            &mut rack,
            session.patch(),
            time,
            vec![],
            true,
            true,
            session.revision(),
        );
        fn has(s: &egui::Shape, t: &str) -> bool {
            match s {
                egui::Shape::Vec(v) => v.iter().any(|s| has(s, t)),
                egui::Shape::Text(s) => s.galley.text() == t,
                _ => false,
            }
        }
        assert!(out.shapes.iter().any(|s| has(&s.shape, expected)));
    }
}
