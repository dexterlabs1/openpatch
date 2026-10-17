use super::*;
use crate::construction_motion::MotionMode;

fn panel_visible(panel: &mut crate::construction_ui::Panel) -> bool {
    let context = egui::Context::default();
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 300.0),
            )),
            ..Default::default()
        },
        |context| assert!(panel.show(context).is_none()),
    );
    !output.shapes.is_empty()
}

fn observe_current(app: &mut OpenPatch) -> Value {
    let state = app.shared.lock().unwrap().construction_snapshot();
    app.construction.observe(state.clone());
    state
}

fn retained_plan(app: &mut OpenPatch, cancelled: bool) -> Value {
    let mut steps = vec![json!({"label":"Add an auditable source","edits":[
        {"tool":"add_module","arguments":{"id":"document_plan_source","kind":"constant"}}
    ]})];
    if cancelled {
        steps.push(json!({"label":"Leave this step pending","edits":[
            {"tool":"set_parameter","arguments":{"id":"document_plan_source","parameter":"value","value":0.5}}
        ]}));
    }
    let started = execute_shared(&app.shared, "start_construction", json!({
        "title":"Work on the previous document",
        "expected_revision":app.revision,"interval_ms":if cancelled { 2000 } else { 100 },"steps":steps
    })).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = app.shared.lock().unwrap().construction_snapshot();
        if state["completed_steps"] == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Construction did not commit: {state}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    if cancelled {
        execute_shared(
            &app.shared,
            "cancel_construction",
            json!({"plan_id":started["plan_id"]}),
        )
        .unwrap();
    }
    let state = observe_current(app);
    assert_eq!(
        state["status"],
        if cancelled { "cancelled" } else { "completed" }
    );
    while app.revision != state["current_revision"].as_u64().unwrap() {
        assert!(
            Instant::now() < deadline,
            "Host did not observe construction"
        );
        app.sync();
        std::thread::yield_now();
    }
    assert!(panel_visible(&mut app.construction));
    state
}

#[test]
fn new_replace_open_import_and_recovery_hide_prior_plans_but_retain_receipts() {
    for cancelled in [false, true] {
        for action in ["new", "replace", "open", "import", "recovery"] {
            let dir = tempfile::tempdir().unwrap();
            let document =
                Document::new(openpatch_core::demo_patch(), Rack::default().layout()).unwrap();
            {
                let store = Store::new(dir.path()).unwrap();
                store.save_new("saved.openpatch", &document).unwrap();
                store.save_recovery(&document).unwrap();
            }
            std::fs::write(
                dir.path().join("legacy.json"),
                serde_json::to_vec(&document.patch()).unwrap(),
            )
            .unwrap();
            let mut app = host(dir.path());
            let recovery = app.recovery_names[0].clone();
            app.rack.set_agent_follow(true);
            app.construction.set_motion_mode(MotionMode::Reduced);
            let previous = retained_plan(&mut app, cancelled);
            let revision = app.revision;
            match action {
                "new" => app.new_from_template(false),
                "replace" => app.replace_now(Patch::new_v2(), "Untitled"),
                "open" => app.load_document("saved.openpatch", false),
                "import" => app.load_document("legacy.json", false),
                "recovery" => app.load_document(&recovery, true),
                _ => unreachable!(),
            }
            assert_eq!(
                app.revision,
                revision + 1,
                "{action} must install exactly one document"
            );
            let retained = execute_shared(
                &app.shared,
                "inspect_construction",
                json!({"plan_id":previous["plan_id"]}),
            )
            .unwrap();
            for key in [
                "plan_id",
                "status",
                "base_revision",
                "completed_steps",
                "last_revision",
                "last_label",
            ] {
                assert_eq!(retained[key], previous[key], "{action} must retain {key}");
            }
            // Receipt age is live metadata; all committed-edit identities and
            // effects must survive the presentation boundary unchanged.
            let mut before_receipt = previous["last_commit"].clone();
            let mut after_receipt = retained["last_commit"].clone();
            before_receipt.as_object_mut().unwrap().remove("age_ms");
            after_receipt.as_object_mut().unwrap().remove("age_ms");
            assert_eq!(after_receipt, before_receipt, "{action} erased a receipt");
            for _ in 0..2 {
                app.sync();
                observe_current(&mut app);
                assert!(
                    !panel_visible(&mut app.construction),
                    "{action} displayed the previous document's plan"
                );
            }
            assert!(app.rack.agent_follow_enabled());
            assert_eq!(app.construction.motion_mode(), MotionMode::Reduced);
        }
    }
}

#[test]
fn same_document_edits_and_new_plans_remain_visible_after_a_document_reset() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    retained_plan(&mut app, false);
    assert!(app.command(
        "set_parameter",
        json!({"id":"document_plan_source","parameter":"value","value":0.25})
    ));
    observe_current(&mut app);
    assert!(
        panel_visible(&mut app.construction),
        "ordinary edits do not start another document"
    );

    app.replace_now(Patch::new_v2(), "Untitled");
    observe_current(&mut app);
    assert!(!panel_visible(&mut app.construction));
    let previous_revision = app.revision;
    let started = execute_shared(
        &app.shared,
        "start_construction",
        json!({
            "expected_revision":previous_revision,"interval_ms":2000,
            "steps":[{"label":"Build the new document","edits":[
                {"tool":"add_module","arguments":{"id":"new_document_source","kind":"constant"}}
            ]}]
        }),
    )
    .unwrap();
    assert_eq!(started["base_revision"], previous_revision);
    observe_current(&mut app);
    assert!(
        panel_visible(&mut app.construction),
        "a new plan is visible before its first edit"
    );
    execute_shared(
        &app.shared,
        "cancel_construction",
        json!({"plan_id":started["plan_id"]}),
    )
    .unwrap();
    observe_current(&mut app);
    assert!(
        panel_visible(&mut app.construction),
        "this document's cancelled plan remains visible"
    );
}

#[test]
fn accepted_document_boundary_preserves_a_new_plan_observed_after_its_first_commit() {
    let mut panel = crate::construction_ui::Panel::default();
    panel.set_motion_mode(MotionMode::Off);
    panel.begin_document(40);
    panel.observe(
        json!({"plan_id":7,"base_revision":39,"status":"completed","current_revision":40}),
    );
    assert!(!panel_visible(&mut panel));
    // A plan can start and commit after replacement, before the desktop's next
    // sync. Scope to the accepted replacement revision, not that later sync.
    panel.observe(json!({"plan_id":8,"base_revision":40,"status":"completed","current_revision":41,"completed_steps":1,"total_steps":1}));
    assert!(panel_visible(&mut panel));
    assert_eq!(panel.motion_mode(), MotionMode::Off);
}

#[test]
fn failed_document_replacement_keeps_the_current_plan_visible_and_auditable() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let previous = retained_plan(&mut app, false);
    let before = app.shared.lock().unwrap().snapshot();
    app.load_document("missing.openpatch", false);
    app.replace_now(
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"bad","kind":"missing_module"}]}))
            .unwrap(),
        "Invalid",
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    let current = observe_current(&mut app);
    assert_eq!(current["plan_id"], previous["plan_id"]);
    assert!(panel_visible(&mut app.construction));
}

#[test]
fn direct_fixture_session_replacement_resets_the_banner_before_a_host_sync() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let previous = retained_plan(&mut app, false);
    let revision = app.revision;
    execute_shared(
        &app.shared,
        "replace_patch",
        json!({"patch":Patch::new_v2(),"expected_revision":revision}),
    )
    .unwrap();
    assert_eq!(app.revision, revision, "fixture has not synced yet");
    app.replace_rack(Rack::default());
    observe_current(&mut app);
    assert!(!panel_visible(&mut app.construction));
    assert_eq!(
        app.shared.lock().unwrap().construction_snapshot()["plan_id"],
        previous["plan_id"]
    );
}

#[test]
fn new_document_decision_pauses_the_current_plan_and_only_scopes_after_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    edit(&mut app, 330.0);
    let started = execute_shared(
        &app.shared,
        "start_construction",
        json!({
            "expected_revision":app.revision,"interval_ms":2000,
            "steps":[{"label":"Pending new-document decision","edits":[
                {"tool":"add_module","arguments":{"id":"pending_source","kind":"constant"}}
            ]}]
        }),
    )
    .unwrap();
    app.request_action(PendingAction::NewTemplate);
    let deadline = Instant::now() + Duration::from_secs(5);
    while let Some(action) = app.deferred_action.take() {
        assert!(Instant::now() < deadline, "New remained deferred");
        app.request_action(action);
        std::thread::yield_now();
    }
    assert!(app.confirm_discard.is_some());
    let paused = observe_current(&mut app);
    assert_eq!(paused["status"], "paused");
    assert!(panel_visible(&mut app.construction));
    // Cancel the document decision. The current plan stays paused and belongs
    // to the current patch until a replacement actually succeeds.
    app.confirm_discard = None;
    app.sync();
    assert_eq!(observe_current(&mut app)["status"], "paused");
    assert!(panel_visible(&mut app.construction));
    app.new_from_template(false);
    let retained = observe_current(&mut app);
    assert_eq!(retained["plan_id"], started["plan_id"]);
    assert_eq!(retained["status"], "conflicted");
    assert!(!panel_visible(&mut app.construction));
    assert!(execute_shared(
        &app.shared,
        "resume_construction",
        json!({
            "plan_id":started["plan_id"],"expected_revision":app.revision
        })
    )
    .is_err());
}

#[test]
fn identical_patch_replacement_hides_a_cancelled_plan_with_no_committed_steps() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let revision = app.revision;
    let started = execute_shared(
        &app.shared,
        "start_construction",
        json!({
            "expected_revision":revision,"interval_ms":2000,
            "steps":[{"label":"Never committed","edits":[
                {"tool":"add_module","arguments":{"id":"never_committed","kind":"constant"}}
            ]}]
        }),
    )
    .unwrap();
    let cancelled = execute_shared(
        &app.shared,
        "cancel_construction",
        json!({"plan_id":started["plan_id"]}),
    )
    .unwrap();
    assert_eq!(cancelled["base_revision"], revision);
    assert_eq!(cancelled["completed_steps"], 0);
    app.construction.observe(cancelled);
    assert!(panel_visible(&mut app.construction));
    app.replace_now(app.patch.clone(), "Same patch, new document");
    assert_eq!(app.revision, revision + 1);
    let retained = observe_current(&mut app);
    assert_eq!(retained["plan_id"], started["plan_id"]);
    assert!(!panel_visible(&mut app.construction));
}
