use super::*;

#[test]
fn actual_direct_and_progressive_moves_animate_without_changing_graph_history_or_layout() {
    use crate::construction_motion::MotionMode;
    for progressive in [false, true] {
        let mut results = Vec::new();
        for mode in [MotionMode::Full, MotionMode::Reduced, MotionMode::Off] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = host(dir.path());
            app.construction.set_motion_mode(mode);
            let origin = Instant::now();
            let receipt = app.prepare_cable_move_fixture(origin, progressive);
            assert_eq!(receipt["moved_cables"].as_array().unwrap().len(), 2);
            assert_eq!(app.patch.cables.len(), 2);
            let tone = app
                .patch
                .cables
                .iter()
                .find(|c| c.id.as_deref() == Some("moving-tone"))
                .unwrap();
            let output = app
                .patch
                .cables
                .iter()
                .find(|c| c.id.as_deref() == Some("moving-output"))
                .unwrap();
            assert_eq!(tone.to, "filter-b");
            assert_eq!(output.from, "filter-b");
            let snapshot = app.shared.lock().unwrap().snapshot();
            let layout = app.rack.layout();
            assert_eq!(
                app.rack
                    .construction_effect_counts_at(origin + Duration::from_millis(180)),
                if mode == MotionMode::Off {
                    (0, 0, 0)
                } else {
                    (0, 2, 0)
                }
            );
            app.sync();
            assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
            assert_eq!(app.rack.layout(), layout);
            assert_eq!(
                app.rack
                    .construction_effect_counts_at(origin + Duration::from_millis(460)),
                (0, 0, 0)
            );
            app.sync();
            assert_eq!(
                app.rack
                    .construction_effect_counts_at(origin + Duration::from_millis(460)),
                (0, 0, 0),
                "an observed receipt restarted after expiry"
            );
            assert!(app.audio.is_none());
            results.push((snapshot, layout));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], results[2]);
    }
}

#[test]
fn manual_move_is_immediate_and_does_not_suppress_the_next_direct_mcp_move() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let origin = Instant::now();
    app.prepare_cable_move_fixture(origin, false);
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 2, 0));
    assert!(app.command(
        "move_cables",
        json!({
            "cable_ids":["moving-tone"],"end":"input","node":"filter-a","port":"signal"
        })
    ));
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 0, 0));
    app.sync();
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 0, 0));
    execute_shared(
        &app.shared,
        "move_cables",
        json!({
            "expected_revision":app.revision,
            "cable_ids":["moving-tone"],"end":"input","node":"filter-b","port":"signal"
        }),
    )
    .unwrap();
    let shared = app.shared.clone();
    let revision = shared.lock().unwrap().revision();
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.revision < revision {
        app.sync();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 1, 0));
    assert!(app.audio.is_none());
}

#[test]
fn locally_acknowledged_move_stays_suppressed_when_sync_was_busy() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let origin = Instant::now();
    app.prepare_cable_move_fixture(origin, false);
    let receipt = execute_shared(
        &app.shared,
        "move_cables",
        json!({
            "expected_revision":app.revision,
            "cable_ids":["moving-tone"],"end":"input","node":"filter-a","port":"signal"
        }),
    )
    .unwrap();
    let revision = receipt["revision"].as_u64().unwrap();
    app.rack.suppress_cable_move_revision(revision);
    let shared = app.shared.clone();
    let guard = shared.lock().unwrap();
    app.sync();
    assert!(app.revision < revision);
    drop(guard);
    app.sync();
    assert_eq!(app.revision, revision);
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 0, 0));
    app.sync();
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 0, 0));
}

#[test]
fn a_new_plan_clears_a_direct_move_without_waiting_for_another_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let origin = Instant::now();
    app.prepare_cable_move_fixture(origin, false);
    assert_eq!(app.rack.construction_effect_counts_at(origin), (0, 2, 0));
    let revision = app.revision;
    let plan = execute_shared(
        &app.shared,
        "start_construction",
        json!({
            "expected_revision":revision,"interval_ms":2000,
            "steps":[{"label":"A new plan","edits":[{"tool":"set_parameter","arguments":{
                "id":"voice","parameter":"frequency","value":220
            }}]}]
        }),
    )
    .unwrap();
    assert!(plan["last_move"].is_null());
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        app.sync();
        assert_eq!(app.revision, revision);
        if app.rack.construction_effect_counts_at(origin) == (0, 0, 0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "new plan did not clear prior direct motion"
        );
        std::thread::yield_now();
    }
    execute_shared(
        &app.shared,
        "cancel_construction",
        json!({"plan_id":plan["plan_id"]}),
    )
    .unwrap();
}

#[test]
fn native_move_capture_phases_use_real_commits_and_request_real_screenshots() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.smoke_test = Some((dir.path().join("unused.png"), 2050, 86));
    let ctx = egui::Context::default();
    app.smoke(&ctx);
    assert_eq!(app.patch.cables.len(), 2);
    let receipt = app.shared.lock().unwrap().construction_snapshot()["last_commit"].clone();
    assert_eq!(receipt["moved_cables"].as_array().unwrap().len(), 2);
    for logical in [2058, 2074, 2090, 2106] {
        app.smoke_test.as_mut().unwrap().1 = logical;
        let output = ctx.run(egui::RawInput::default(), |ctx| app.smoke(ctx));
        assert!(output.viewport_output.values().any(|viewport| viewport
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Screenshot(_)))));
    }
    app.smoke_test.as_mut().unwrap().1 = 2114;
    app.smoke(&ctx);
    assert_eq!(app.rack.cable_presentation().tension, 1.0);
    assert!(!app.browser_open);
    app.smoke_test.as_mut().unwrap().1 = 2122;
    let output = ctx.run(egui::RawInput::default(), |ctx| app.smoke(ctx));
    assert!(output.viewport_output.values().any(|viewport| viewport
        .commands
        .iter()
        .any(|command| matches!(command, egui::ViewportCommand::Screenshot(_)))));
    assert_eq!(
        app.smoke_test.as_ref().unwrap().2,
        86,
        "a screenshot request is not an actual capture receipt"
    );
    assert!(app.audio.is_none());
}
