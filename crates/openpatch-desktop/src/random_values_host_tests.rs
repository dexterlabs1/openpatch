fn random_values_host_patch() -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"trigger","kind":"push"},{"id":"random","kind":"random_values","params":{"seed":42,"range":3}}],
        "cables":[{"from":"trigger","output":"gate","to":"random","input":"trig"}],
        "output":"random","output_port":"out1"})).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}
fn random_values_host_state(callback: &Callback, id: &str) -> openpatch_core::SavedModuleState {
    let engine = &callback.graphs.active.engine;
    engine
        .module_saved_state(engine.resolve_module_state(id).unwrap())
        .unwrap()
}
fn random_values_host_publish(
    callback: &mut Callback,
    updates: &mut Producer<PreparedGraph>,
    disposal: &mut Consumer<PreparedGraph>,
    session: &openpatch::control::Session,
) {
    assert!(updates
        .push(sequence_graph(
            callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
}
fn random_values_host_trigger(callback: &mut Callback) {
    callback
        .performance
        .trigger("trigger", ModuleAction::PushRelease)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
    callback
        .performance
        .trigger("trigger", ModuleAction::PushPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
    callback
        .performance
        .trigger("trigger", ModuleAction::PushRelease)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
}

#[test]
fn random_values_host_live_electrical_trig_capture_save_reopen_and_next_rng_draw() {
    let patch = random_values_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone()));
    assert!(session.capture_project_patch(None).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let initial = random_values_host_state(&callback, "random");
    random_values_host_trigger(&mut callback);
    let triggered = random_values_host_state(&callback, "random");
    assert_ne!(triggered, initial);
    let (
        openpatch_core::SavedModuleState::RandomValues {
            values: before,
            rng: rng_before,
        },
        openpatch_core::SavedModuleState::RandomValues {
            values: after,
            rng: rng_after,
        },
    ) = (initial, triggered)
    else {
        panic!("Random Values state")
    };
    assert_ne!(rng_before, rng_after);
    for output in 0..7 {
        assert_ne!(before[output][0], after[output][0]);
        assert_eq!(before[output][1..], after[output][1..]);
    }
    let snapshot = callback.performance.snapshot();
    assert_eq!(snapshot["modules"]["random"]["state"]["active_channels"], 1);
    assert_eq!(
        snapshot["modules"]["random"]["state"]["rng_state"],
        rng_after
    );
    assert_eq!(
        snapshot["modules"]["random"]["state"]["values"][0][0],
        serde_json::json!(f32::from_bits(after[0][0]))
    );
    let before_session = session.snapshot();
    let saved = session.capture_project_patch(Some(1)).unwrap();
    assert_eq!(saved.saved_states["random"], triggered);
    let preset = session
        .execute("capture_module_preset", serde_json::json!({"id":"random"}))
        .unwrap();
    assert_eq!(preset["preset"]["params"].as_object().unwrap().len(), 116);
    session
        .execute("save_patch", serde_json::json!({"filename":"latest.json"}))
        .unwrap();
    assert_eq!(session.snapshot(), before_session);
    assert_eq!(callback.performance.snapshot(), snapshot);
    let disk: Patch =
        serde_json::from_slice(&std::fs::read(dir.path().join("latest.json")).unwrap()).unwrap();
    assert_eq!(
        openpatch_core::SavedModuleState::from_node(&disk.nodes[1]),
        Some(triggered)
    );
    let (mut reopened, _, _) = sequence_callback(&disk);
    without_heap_activity(|| reopened.render(&mut [0.0_f32; 1], 1));
    assert_eq!(random_values_host_state(&reopened, "random"), triggered);
    random_values_host_trigger(&mut callback);
    random_values_host_trigger(&mut reopened);
    assert_eq!(
        random_values_host_state(&reopened, "random"),
        random_values_host_state(&callback, "random")
    );
}

#[test]
fn random_values_host_range_initialize_seed_randomize_and_history_use_actual_state() {
    let patch = random_values_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone()));
    random_values_host_trigger(&mut callback);
    let actual = random_values_host_state(&callback, "random");
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"random","parameter":"range","value":5}),
        )
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), actual);
    session
        .execute("initialize_modules", serde_json::json!({"ids":["random"]}))
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), actual);
    assert_eq!(session.patch().nodes[1].params["range"], 0.0);
    assert_eq!(session.patch().nodes[1].params["seed"], 1.0);
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"random","parameter":"seed","value":7}),
        )
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let seeded = random_values_host_state(&callback, "random");
    assert_ne!(seeded, actual);
    let mut fresh = session.patch().nodes[1].clone();
    fresh
        .params
        .retain(|name, _| name == "seed" || name == "range");
    assert_eq!(
        seeded,
        openpatch_core::initial_random_values_state(&fresh).unwrap()
    );
    let module_seed = "random".bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    }) ^ 123;
    let openpatch_core::SavedModuleState::RandomValues { values, .. } = seeded else {
        unreachable!()
    };
    let expected = openpatch_core::randomize_random_values_state(
        &session.patch().nodes[1],
        openpatch_core::SavedModuleState::RandomValues {
            values,
            rng: module_seed.max(1),
        },
    )
    .unwrap();
    let before_history = session
        .execute("inspect_history", serde_json::json!({}))
        .unwrap()["history"]["undo_steps"]
        .as_u64()
        .unwrap();
    session
        .execute(
            "randomize_modules",
            serde_json::json!({"ids":["random"],"seed":123}),
        )
        .unwrap();
    assert_eq!(
        session
            .execute("inspect_history", serde_json::json!({}))
            .unwrap()["history"]["undo_steps"],
        before_history + 1
    );
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), expected);
    session.execute("undo", serde_json::json!({})).unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), seeded);
    session.execute("redo", serde_json::json!({})).unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), expected);
    session.execute("undo", serde_json::json!({})).unwrap();
    session.execute("undo", serde_json::json!({})).unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), actual);
    // Identical seed, ID and range produce identical draws despite newer live RNG.
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"random","parameter":"range","value":0}),
        )
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    random_values_host_trigger(&mut callback);
    session
        .execute(
            "randomize_modules",
            serde_json::json!({"ids":["random"],"seed":123}),
        )
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), expected);
}

#[test]
fn random_values_host_duplicate_remove_and_equal_configured_preset_load_restore_live_values() {
    let patch = random_values_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone()));
    random_values_host_trigger(&mut callback);
    let actual = random_values_host_state(&callback, "random");
    let preset = session
        .execute("capture_module_preset", serde_json::json!({"id":"random"}))
        .unwrap()["preset"]
        .clone();
    session
        .execute(
            "apply_module_preset",
            serde_json::json!({"id":"random","preset":preset}),
        )
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    random_values_host_trigger(&mut callback);
    assert_ne!(random_values_host_state(&callback, "random"), actual);
    session
        .execute(
            "apply_module_preset",
            serde_json::json!({"id":"random","preset":preset}),
        )
        .unwrap();
    // Same configured snapshot must still override newer runtime values.
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "random"), actual);
    session
        .execute(
            "duplicate_modules",
            serde_json::json!({"modules":[{"id":"random","new_id":"copy"}]}),
        )
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "copy"), actual);
    session
        .execute("remove_module", serde_json::json!({"id":"copy"}))
        .unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    session.execute("undo", serde_json::json!({})).unwrap();
    random_values_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(random_values_host_state(&callback, "copy"), actual);
}

#[test]
fn random_values_host_failed_coherent_captures_and_history_are_atomic() {
    struct Snapshot(std::sync::Mutex<serde_json::Value>);
    impl LiveModuleEvents for Snapshot {
        fn snapshot(&self) -> serde_json::Value {
            self.0.lock().unwrap().clone()
        }
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<serde_json::Value, String> {
            unreachable!()
        }
    }
    let patch = random_values_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    random_values_host_trigger(&mut callback);
    let coherent = callback.performance.snapshot();
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    let backend = Arc::new(Snapshot(std::sync::Mutex::new(coherent.clone())));
    session.set_live_module_events(Some(backend.clone()));
    let before = session.snapshot();
    for bad in ["coherent", "frame", "revision", "voltage", "rows", "rng"] {
        let mut value = coherent.clone();
        match bad {
            "coherent" => value["coherent"] = serde_json::json!(false),
            "frame" => value["modules"]["random"]["frame"] = serde_json::json!(12345),
            "revision" => value["modules"]["random"]["revision"] = serde_json::json!(0),
            "voltage" => {
                value["modules"]["random"]["state"]["values"][6][15] = serde_json::json!(11)
            }
            "rows" => value["modules"]["random"]["state"]["values"] = serde_json::json!([]),
            "rng" => value["modules"]["random"]["state"]["rng_state"] = serde_json::json!(0),
            _ => unreachable!(),
        }
        *backend.0.lock().unwrap() = value.clone();
        assert!(session.capture_project_patch(None).is_err(), "{bad}");
        assert!(
            session
                .execute("save_patch", serde_json::json!({"filename":"bad.json"}))
                .is_err(),
            "{bad}"
        );
        // Edits capture only their affected stateful modules; mixed-frame validation
        // needs two module captures, while project capture always spans both.
        if bad != "frame" {
            for (name, args) in [
                (
                    "randomize_modules",
                    serde_json::json!({"ids":["random"],"seed":1}),
                ),
                (
                    "duplicate_modules",
                    serde_json::json!({"modules":[{"id":"random","new_id":"copy"}]}),
                ),
                ("remove_module", serde_json::json!({"id":"random"})),
                ("capture_module_preset", serde_json::json!({"id":"random"})),
            ] {
                assert!(session.execute(name, args).is_err(), "{bad}: {name}");
            }
        } else {
            assert!(session
                .execute(
                    "initialize_modules",
                    serde_json::json!({"ids":["random","trigger"]})
                )
                .is_err());
        }
        assert_eq!(session.snapshot(), before);
        assert_eq!(*backend.0.lock().unwrap(), value);
        assert!(!dir.path().join("bad.json").exists());
    }
}

#[test]
fn random_values_host_dirty_native_save_recovery_and_reopen_keep_volts_and_rng() {
    use crate::{
        document_state::DocumentState,
        project::{Document, Store},
        rack::Rack,
    };
    let patch = random_values_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone()));
    let mut rack = Rack::default();
    rack.reconcile(session.patch(), &openpatch_core::catalog());
    let mut dirty = DocumentState::new(session.patch(), session.revision(), &rack);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let initial = session.capture_project_running(None).unwrap();
    assert!(!dirty.observe_saved_runtime(
        session.patch(),
        session.revision(),
        &rack,
        Some(initial.saved_states)
    ));
    random_values_host_trigger(&mut callback);
    let actual = session.capture_project_patch(None).unwrap();
    assert!(dirty.observe_saved_runtime(
        session.patch(),
        session.revision(),
        &rack,
        Some(actual.saved_states.clone())
    ));
    assert!(dirty.dirty());
    let document = Document::new(actual.patch, rack.layout()).unwrap();
    let store = Store::new(dir.path()).unwrap();
    store.save_new("random.openpatch", &document).unwrap();
    store.save_recovery(&document).unwrap();
    dirty.mark_saved(&document);
    assert!(!dirty.dirty());
    drop(store);
    let store = Store::new(dir.path()).unwrap();
    let recovered = store
        .load_recovery(&store.recoveries()[0])
        .unwrap()
        .document;
    let loaded = store.load("random.openpatch").unwrap().document;
    assert!(loaded.equivalent(&recovered));
    let state = random_values_host_state(&callback, "random");
    for document in [loaded, recovered] {
        let (mut reopened, _, _) = sequence_callback(&document.patch);
        without_heap_activity(|| reopened.render(&mut [0.0_f32; 2], 1));
        assert_eq!(random_values_host_state(&reopened, "random"), state);
        random_values_host_trigger(&mut reopened);
        assert_ne!(random_values_host_state(&reopened, "random"), state);
    }
    assert!(dirty.observe_saved_runtime(session.patch(), session.revision(), &rack, None));
    assert!(
        dirty.dirty(),
        "unavailable state is never mistaken for saved state"
    );
    assert!(dirty.observe_saved_runtime(
        session.patch(),
        session.revision(),
        &rack,
        Some(actual.saved_states)
    ));
    assert!(!dirty.dirty());
}

#[test]
fn random_values_host_saved_load_fences_old_actions_and_stale_plans_without_replay() {
    let patch = random_values_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    random_values_host_trigger(&mut callback);
    let saved = random_values_host_state(&callback, "random");
    let stale = sequence_graph(&callback, &patch, 1);
    callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .unwrap();
    callback.performance.sync_patch(&patch, 2);
    callback
        .performance
        .load_module_saved_states(&[("random".into(), saved)].into());
    callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .unwrap();
    assert!(updates.push(stale).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    assert_eq!(callback.performance.snapshot()["rejected"], 1);
    assert_eq!(callback.performance.snapshot()["pending"], 1);
    while disposal.pop().is_ok() {}
    let older = sequence_graph(&callback, &patch, 1);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    let after = random_values_host_state(&callback, "random");
    assert_ne!(
        after, saved,
        "new action applies after explicit state assignment"
    );
    while disposal.pop().is_ok() {}
    assert!(updates.push(older).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    assert_eq!(
        random_values_host_state(&callback, "random"),
        after,
        "state load is not replayed"
    );
    while disposal.pop().is_ok() {}
    callback.performance.sync_patch(&patch, 3);
    callback
        .performance
        .load_module_saved_states(&[("random".into(), saved)].into());
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    for graph in [
        &callback.graphs.active,
        callback.graphs.previous.as_ref().unwrap(),
    ] {
        let handle = graph.engine.resolve_module_state("random").unwrap();
        assert_eq!(graph.engine.module_saved_state(handle), Some(saved));
    }
}
