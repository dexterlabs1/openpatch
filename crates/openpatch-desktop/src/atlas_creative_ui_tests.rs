use super::*;
#[test]
fn creative_mcp_cues_cover_all_eighteen_rotaries_and_both_mode_controls() {
    use crate::construction_motion::MotionMode;
    let catalog = openpatch_core::catalog();
    for kind in ["macro_voice", "drum_voice", "sidechain_compressor"] {
        let module = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        let mut before: Patch =
            serde_json::from_value(json!({"version":2,"nodes":[{"id":"voice","kind":kind}]}))
                .unwrap();
        let mut after = before.clone();
        let names: Vec<_> = module["params"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let changes:Vec<_>=names.iter().map(|name| {
            let spec=&module["params"][name];
            let min=spec["min"].as_f64().unwrap() as f32;
            let max=spec["max"].as_f64().unwrap() as f32;
            let choice=spec["step"].is_number();
            let old=if choice {min} else {min+(max-min)*0.25};
            let new=if choice {max} else {min+(max-min)*0.75};
            before.nodes[0].params.insert(name.clone(),old);
            after.nodes[0].params.insert(name.clone(),new);
            json!({"id":"voice","kind":kind,"parameter":name,"old_effective":old,"new_effective":new})
        }).collect();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
        let now = std::time::Instant::now();
        let receipt = json!({"plan_id":24,"completed_steps":1,"last_revision":10,"current_revision":10,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":24,"step_index":0,"before_revision":9,"revision":10,"label":"Shape voice","new_modules":[],"new_cables":[],"parameter_changes":changes}});
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
            let anchor = &rack.construction_anchors[&("voice".into(), name.clone())];
            assert_ne!(anchor.old_text, anchor.new_text);
            if matches!(name.as_str(), "engine" | "mode") {
                assert!(anchor.marker.is_none());
                assert_eq!(
                    (&*anchor.old_text, &*anchor.new_text),
                    if kind == "macro_voice" {
                        ("Analog", "Metallic")
                    } else {
                        ("Kick", "Hi-hat")
                    }
                );
            } else {
                let Some(crate::construction_motion::Marker::Rotary {
                    center, old, new, ..
                }) = &anchor.marker
                else {
                    panic!("{kind}.{name}")
                };
                assert!(old < new && (0.0..=1.0).contains(old) && (0.0..=1.0).contains(new));
                assert!(anchor.rect.contains(*center));
                assert!(
                    center.distance(atlas_creative_control(kind, &name).0 + vec2(24.0, 12.0))
                        < 0.001
                );
            }
        }
    }
}
#[test]
fn compressor_configured_curve_agrees_with_actual_settled_engine() {
    for knee in [0.0, 6.0, 24.0] {
        for ratio in [1.0, 4.0, 20.0] {
            for input_db in [-24.0, -13.5, -10.5, -7.5, 6.0] {
                let input = (5.0 * 10.0_f64.powf(input_db / 20.0)) as f32;
                let patch:Patch=serde_json::from_value(json!({"version":2,"sample_rate":8000,"nodes":[
                    {"id":"dc","kind":"constant","params":{"value":input}},
                    {"id":"fx","kind":"sidechain_compressor","params":{"threshold_db":-10.5,"ratio":ratio,"knee_db":knee,"makeup_db":3.0,"attack_ms":0.1,"release_ms":5.0,"detector_ms":0.1}}],
                    "cables":[{"from":"dc","output":"out","to":"fx","input":"left"}],"output":"fx","output_port":"left"})).unwrap();
                let mut engine = patch.compile().unwrap();
                let mut output = [0.0; 4096];
                engine.process_block(&mut output);
                let actual_db = 20.0 * (f64::from(output[4095].abs()) / 5.0).log10();
                let expected_db =
                    atlas_compressor_guide_target_db(input_db, -10.5, ratio, knee) + 3.0;
                assert!((actual_db-expected_db).abs()<0.001,"knee={knee} ratio={ratio} input={input_db}: actual={actual_db} expected={expected_db}");
            }
        }
    }
}
#[test]
fn drum_source_and_envelope_legend_follow_actual_session_choices_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session
        .replace_patch(
            serde_json::from_value(
                json!({"version":2,"nodes":[{"id":"voice","kind":"drum_voice"}]}),
            )
            .unwrap(),
            None,
        )
        .unwrap();
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    for (time, source, t60) in [
        (0.0, "SINE", "ENV T60 200 ms"),
        (0.1, "NOISE", "ENV T60 200 ms"),
        (0.2, "METAL", "ENV T60 200 ms"),
        (0.3, "METAL", "ENV T60 1000 ms"),
        (0.4, "METAL", "ENV T60 200 ms"),
    ] {
        if time == 0.1 || time == 0.2 {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"voice","parameter":"mode","value":if time==0.1 {1} else {2}}),
                )
                .unwrap();
        }
        if time == 0.3 {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"voice","parameter":"decay","value":1}),
                )
                .unwrap();
        }
        if time == 0.4 {
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
        for expected in [source, t60] {
            assert!(
                out.shapes.iter().any(|s| has(&s.shape, expected)),
                "{expected}"
            );
        }
    }
}
