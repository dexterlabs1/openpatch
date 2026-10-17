use super::*;

#[test]
fn atlas_scope_rotaries_commit_successive_session_moves_and_one_undo() {
    for (index, parameter, initial) in [
        (2, "time", 0.5_f32),
        (4, "x_offset", 0.0),
        (5, "y_offset", 0.0),
        (6, "threshold", 0.0),
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"scope","kind":"scope",
                "params":{parameter:initial,"auto_range":1}}]
        }))
        .unwrap();
        session.replace_patch(patch, None).unwrap();
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let center = scope_knob_center(index) + vec2(24.0, 12.0);
        let baseline = session.snapshot()["history"]["undo_steps"]
            .as_u64()
            .unwrap();
        let mut draw = |session: &mut openpatch::control::Session, time, events| {
            let revision = session.revision();
            let action = frame_revision_state(
                &ctx,
                &mut rack,
                session.patch(),
                time,
                events,
                true,
                true,
                revision,
            )
            .0;
            action.map(|action| {
                let RackAction::Command(tool, args) = action else {
                    panic!("Scope rotary must emit a parameter edit")
                };
                assert_eq!(args["expected_revision"], revision);
                session.execute(tool, args.clone()).unwrap();
                (tool, args)
            })
        };
        draw(&mut session, 0.0, vec![]);
        assert!(draw(
            &mut session,
            1.0,
            vec![egui::Event::PointerMoved(center), button(center, true)],
        )
        .is_none());
        let first = draw(
            &mut session,
            1.1,
            vec![egui::Event::PointerMoved(center - vec2(0.0, 15.0))],
        )
        .expect("first rotary move");
        let manual_axis = matches!(parameter, "x_offset" | "y_offset");
        assert_eq!(
            first.0,
            if manual_axis {
                "apply_edits"
            } else {
                "set_parameter"
            }
        );
        let group = first.1["history_group"].as_str().unwrap().to_owned();
        let second = draw(
            &mut session,
            1.2,
            vec![egui::Event::PointerMoved(center - vec2(0.0, 30.0))],
        )
        .expect("second rotary move must use the new Session revision");
        assert_eq!(second.0, "set_parameter");
        assert_eq!(second.1["parameter"], parameter);
        assert_eq!(second.1["history_group"], group);
        let value = session.patch().nodes[0].params[parameter];
        let expected = if parameter == "time" {
            initial * 10.0_f32.powf(0.8)
        } else {
            4.0
        };
        assert!((value - expected).abs() < 1e-5, "{parameter}: {value}");
        assert_eq!(
            session.patch().nodes[0].params["auto_range"],
            if manual_axis { 0.0 } else { 1.0 }
        );
        draw(
            &mut session,
            1.3,
            vec![button(center - vec2(0.0, 30.0), false)],
        );
        assert_eq!(
            session.snapshot()["history"]["undo_steps"]
                .as_u64()
                .unwrap(),
            baseline + 1
        );
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.patch().nodes[0].params[parameter], initial);
        assert_eq!(session.patch().nodes[0].params["auto_range"], 1.0);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.patch().nodes[0].params[parameter], value);
    }
}

#[test]
fn atlas_scope_time_rotary_uses_literal_seconds_at_keyboard_boundaries() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({
        "version":2,"nodes":[{"id":"scope","kind":"scope","params":{"time":0.5}}]
    }))
    .unwrap();
    let center = scope_knob_center(2) + vec2(24.0, 12.0);
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    frame(
        &ctx,
        &mut rack,
        &patch,
        1.0,
        vec![egui::Event::PointerMoved(center), button(center, true)],
    );
    frame(&ctx, &mut rack, &patch, 1.1, vec![button(center, false)]);
    for (step, key, value) in [(0, egui::Key::Home, 0.005), (1, egui::Key::End, 50.0)] {
        let action = frame(
            &ctx,
            &mut rack,
            &patch,
            2.0 + f64::from(step),
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let Some(RackAction::Command("set_parameter", args)) = action else {
            panic!("focused TIME rotary must accept Home/End")
        };
        assert_eq!(args["parameter"], "time");
        assert!((args["value"].as_f64().unwrap() - value).abs() < 1e-8);
    }
}
