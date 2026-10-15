fn shift_register_host_patch() -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"trigger","kind":"push"},{"id":"random","kind":"sh_asr","params":{"seed":42,"range":3,"random":1}}],
        "cables":[{"from":"trigger","output":"gate","to":"random","input":"trig1"}],
        "output":"random","output_port":"out1"})).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}
fn shift_register_host_state(callback: &Callback, id: &str) -> openpatch_core::SavedModuleState {
    let engine = &callback.graphs.active.engine;
    engine
        .module_saved_state(engine.resolve_module_state(id).unwrap())
        .unwrap()
}
fn shift_register_host_publish(
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
fn shift_register_host_trigger(callback: &mut Callback) {
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
fn shift_register_host_live_electrical_trig_capture_save_reopen_and_next_rng_draw() {
    let patch = shift_register_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    let before = session.snapshot();
    session
        .execute(
            "randomize_modules",
            serde_json::json!({"ids":["random"],"seed":1}),
        )
        .unwrap();
    assert_eq!(
        session.snapshot(),
        before,
        "disabled Randomize needs no live telemetry"
    );
    assert!(session.capture_project_patch(None).is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let initial = shift_register_host_state(&callback, "random");
    shift_register_host_trigger(&mut callback);
    let triggered = shift_register_host_state(&callback, "random");
    assert_ne!(triggered, initial);
    let (
        openpatch_core::SavedModuleState::ShiftRegister {
            values: before,
            rng: rng_before,
        },
        openpatch_core::SavedModuleState::ShiftRegister {
            values: after,
            rng: rng_after,
        },
    ) = (initial, triggered)
    else {
        panic!("Shift Register state")
    };
    assert_ne!(rng_before, rng_after);
    assert_ne!(before[0], after[0]);
    assert_eq!(after[1..], before[..7]);
    let snapshot = callback.performance.snapshot();
    assert_eq!(
        snapshot["modules"]["random"]["state"]["rng_state"],
        rng_after
    );
    assert_eq!(
        snapshot["modules"]["random"]["state"]["values"][0],
        serde_json::json!(f32::from_bits(after[0]))
    );
    let before_session = session.snapshot();
    let saved = session.capture_project_patch(Some(1)).unwrap();
    assert_eq!(saved.saved_states["random"], triggered);
    let preset = session
        .execute("capture_module_preset", serde_json::json!({"id":"random"}))
        .unwrap();
    assert_eq!(preset["preset"]["params"].as_object().unwrap().len(), 13);
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
    assert_eq!(shift_register_host_state(&reopened, "random"), triggered);
    shift_register_host_trigger(&mut callback);
    shift_register_host_trigger(&mut reopened);
    assert_eq!(
        shift_register_host_state(&reopened, "random"),
        shift_register_host_state(&callback, "random")
    );
}

#[test]
fn shift_register_host_range_initialize_seed_randomize_and_history_use_actual_state() {
    let patch = shift_register_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    shift_register_host_trigger(&mut callback);
    let actual = shift_register_host_state(&callback, "random");
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"random","parameter":"range","value":5}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "random"), actual);
    callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .unwrap();
    callback
        .performance
        .trigger("random", ModuleAction::ResetPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let actual = shift_register_host_state(&callback, "random");
    let before = session.snapshot();
    let controls = callback.performance.snapshot();
    assert_ne!(controls["modules"]["random"]["controls"]["pressed_mask"], 0);
    session
        .execute(
            "randomize_modules",
            serde_json::json!({"ids":["random"],"seed":123}),
        )
        .unwrap();
    assert_eq!(
        session.snapshot(),
        before,
        "entire-module Randomize is a no-op"
    );
    assert_eq!(callback.performance.snapshot(), controls);
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"random","parameter":"seed","value":7}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let seeded = shift_register_host_state(&callback, "random");
    let (
        openpatch_core::SavedModuleState::ShiftRegister {
            values: old,
            rng: old_rng,
        },
        openpatch_core::SavedModuleState::ShiftRegister {
            values: new,
            rng: new_rng,
        },
    ) = (actual, seeded)
    else {
        unreachable!()
    };
    assert_eq!(old, new);
    assert_ne!(old_rng, new_rng);
    assert_eq!(new_rng, 7 ^ 0xA341316C);
    callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let before_initialize = shift_register_host_state(&callback, "random");
    assert!(
        callback.performance.snapshot()["modules"]["random"]["controls"]["pressed"]
            .as_bool()
            .unwrap()
    );
    session
        .execute("initialize_modules", serde_json::json!({"ids":["random"]}))
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(
        shift_register_host_state(&callback, "random"),
        openpatch_core::SavedModuleState::ShiftRegister {
            values: [0.0_f32.to_bits(); 8],
            rng: 1 ^ 0xA341316C
        }
    );
    assert_eq!(session.patch().nodes[1].params["random"], 0.0);
    assert_eq!(session.patch().nodes[1].params["range"], 0.0);
    assert_eq!(session.patch().nodes[1].params["seed"], 1.0);
    assert_eq!(
        callback.performance.snapshot()["modules"]["random"]["controls"]["pressed"],
        false
    );
    session.execute("undo", serde_json::json!({})).unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(
        shift_register_host_state(&callback, "random"),
        before_initialize
    );
    session.execute("redo", serde_json::json!({})).unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(
        shift_register_host_state(&callback, "random"),
        openpatch_core::SavedModuleState::ShiftRegister {
            values: [0; 8],
            rng: 1 ^ 0xA341316C
        }
    );
}

#[test]
fn shift_register_host_duplicate_remove_and_equal_configured_preset_load_restore_live_values() {
    let patch = shift_register_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    shift_register_host_trigger(&mut callback);
    let actual = shift_register_host_state(&callback, "random");
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
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    shift_register_host_trigger(&mut callback);
    assert_ne!(shift_register_host_state(&callback, "random"), actual);
    session
        .execute(
            "apply_module_preset",
            serde_json::json!({"id":"random","preset":preset}),
        )
        .unwrap();
    // Same configured snapshot must still override newer runtime values.
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "random"), actual);
    session
        .execute(
            "duplicate_modules",
            serde_json::json!({"modules":[{"id":"random","new_id":"copy"}]}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "copy"), actual);
    session
        .execute("remove_module", serde_json::json!({"id":"copy"}))
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    session.execute("undo", serde_json::json!({})).unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "copy"), actual);
}

#[test]
fn shift_register_host_failed_coherent_captures_and_history_are_atomic() {
    struct Snapshot(std::sync::Mutex<serde_json::Value>);
    impl LiveModuleEvents for Snapshot {
        fn snapshot(&self) -> serde_json::Value {
            self.0.lock().unwrap().clone()
        }
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<serde_json::Value, String> {
            unreachable!()
        }
    }
    let patch = shift_register_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    shift_register_host_trigger(&mut callback);
    let coherent = callback.performance.snapshot();
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    let backend = Arc::new(Snapshot(std::sync::Mutex::new(coherent.clone())));
    session.set_live_module_events(Some(backend.clone())).unwrap();
    let before = session.snapshot();
    for bad in ["coherent", "frame", "revision", "voltage", "rows", "rng"] {
        let mut value = coherent.clone();
        match bad {
            "coherent" => value["coherent"] = serde_json::json!(false),
            "frame" => value["modules"]["random"]["frame"] = serde_json::json!(12345),
            "revision" => value["modules"]["random"]["revision"] = serde_json::json!(0),
            "voltage" => {
                value["modules"]["random"]["state"]["values"][7] = serde_json::json!(1_000_001)
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
                ("initialize_modules", serde_json::json!({"ids":["random"]})),
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
fn shift_register_host_dirty_native_save_recovery_and_reopen_keep_volts_and_rng() {
    use crate::{
        document_state::DocumentState,
        project::{Document, Store},
        rack::Rack,
    };
    let patch = shift_register_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
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
    shift_register_host_trigger(&mut callback);
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
    store.save_new("shift.openpatch", &document).unwrap();
    store.save_recovery(&document).unwrap();
    dirty.mark_saved(&document);
    assert!(!dirty.dirty());
    drop(store);
    let store = Store::new(dir.path()).unwrap();
    let recovered = store
        .load_recovery(&store.recoveries()[0])
        .unwrap()
        .document;
    let loaded = store.load("shift.openpatch").unwrap().document;
    assert!(loaded.equivalent(&recovered));
    let state = shift_register_host_state(&callback, "random");
    shift_register_host_trigger(&mut callback);
    let expected_next = shift_register_host_state(&callback, "random");
    for document in [loaded, recovered] {
        let (mut reopened, _, _) = sequence_callback(&document.patch);
        without_heap_activity(|| reopened.render(&mut [0.0_f32; 2], 1));
        assert_eq!(shift_register_host_state(&reopened, "random"), state);
        shift_register_host_trigger(&mut reopened);
        assert_eq!(
            shift_register_host_state(&reopened, "random"),
            expected_next
        );
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
fn shift_register_host_saved_load_fences_old_actions_and_stale_plans_without_replay() {
    let patch = shift_register_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    shift_register_host_trigger(&mut callback);
    let saved = shift_register_host_state(&callback, "random");
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
    let after = shift_register_host_state(&callback, "random");
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
        shift_register_host_state(&callback, "random"),
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

#[test]
fn shift_register_host_seed_only_preset_bypass_and_initialize_preserve_detector_history() {
    let patch = shift_register_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    shift_register_host_trigger(&mut callback);
    let before = shift_register_host_state(&callback, "random");
    session
        .execute(
            "apply_module_preset",
            serde_json::json!({"id":"random","preset":{
                "format":"openpatch-module-preset","version":1,"kind":"sh_asr","params":{"seed":123}
            }}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let openpatch_core::SavedModuleState::ShiftRegister { values, rng } =
        shift_register_host_state(&callback, "random")
    else {
        unreachable!()
    };
    let openpatch_core::SavedModuleState::ShiftRegister {
        values: before_values,
        ..
    } = before
    else {
        unreachable!()
    };
    assert_eq!(values, before_values);
    assert_eq!(rng, 123 ^ 0xA341316C);
    callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let held = shift_register_host_state(&callback, "random");
    session
        .execute(
            "set_bypass",
            serde_json::json!({"id":"random","bypassed":true}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "random"), held);
    assert_eq!(
        callback.performance.snapshot()["modules"]["random"]["controls"]["pressed"],
        false
    );
    assert!(callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .is_err());
    shift_register_host_trigger(&mut callback);
    assert_eq!(shift_register_host_state(&callback, "random"), held);
    session
        .execute(
            "set_bypass",
            serde_json::json!({"id":"random","bypassed":false}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "random"), held);
    // Establish a high CV detector and keep the source physically held through
    // Initialize. Re-enabling random afterward must not create a phantom rise.
    callback
        .performance
        .trigger("trigger", ModuleAction::PushPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    session
        .execute("initialize_modules", serde_json::json!({"ids":["random"]}))
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let cleared = shift_register_host_state(&callback, "random");
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"random","parameter":"random","value":1}),
        )
        .unwrap();
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert_eq!(shift_register_host_state(&callback, "random"), cleared);
    shift_register_host_trigger(&mut callback);
    assert_ne!(shift_register_host_state(&callback, "random"), cleared);
}

#[test]
fn shift_register_host_snapshot_presets_require_all_ten_fields_and_bounded_values() {
    let patch = shift_register_host_patch();
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    let preset = session
        .execute("capture_module_preset", serde_json::json!({"id":"random"}))
        .unwrap()["preset"]
        .clone();
    for field in ["value1", "value8", "rng_hi", "rng_lo"] {
        let mut partial = preset.clone();
        partial["params"].as_object_mut().unwrap().remove(field);
        assert!(openpatch::presets::validate(&partial).is_err(), "{field}");
    }
    for invalid in [
        serde_json::json!(1_000_001),
        serde_json::json!(-1_000_001),
        serde_json::json!("NaN"),
    ] {
        let mut bad = preset.clone();
        bad["params"]["value8"] = invalid;
        assert!(openpatch::presets::validate(&bad).is_err());
    }
    let mut boundary = preset.clone();
    boundary["params"]["value1"] = serde_json::json!(-1_000_000);
    boundary["params"]["value8"] = serde_json::json!(1_000_000);
    openpatch::presets::validate(&boundary).unwrap();
    let mut zero_rng = preset;
    zero_rng["params"]["rng_hi"] = serde_json::json!(0);
    zero_rng["params"]["rng_lo"] = serde_json::json!(0);
    assert!(openpatch::presets::validate(&zero_rng).is_err());
}

#[test]
fn shift_register_host_queued_push_clear_controls_are_ordered_and_clear_keeps_draws() {
    let patch = shift_register_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let openpatch_core::SavedModuleState::ShiftRegister {
        rng: mut expected, ..
    } = shift_register_host_state(&callback, "random")
    else {
        unreachable!()
    };
    for _ in 0..2 {
        expected ^= expected << 13;
        expected ^= expected >> 17;
        expected ^= expected << 5;
    }
    for action in [
        ModuleAction::PushPress,
        ModuleAction::PushPress,
        ModuleAction::PushRelease,
        ModuleAction::PushPress,
        ModuleAction::PushRelease,
        ModuleAction::ResetPress,
        ModuleAction::ResetRelease,
    ] {
        callback.performance.trigger("random", action).unwrap();
    }
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        shift_register_host_state(&callback, "random"),
        openpatch_core::SavedModuleState::ShiftRegister {
            values: [0; 8],
            rng: expected
        }
    );
    let snapshot = callback.performance.snapshot();
    assert_eq!(snapshot["modules"]["random"]["controls"]["pressed_mask"], 0);
    assert_eq!(snapshot["applied"], 7);
}

#[test]
fn shift_register_host_initialize_batch_retains_later_random_range_and_seed_edits() {
    let patch = shift_register_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let dir = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(dir.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    shift_register_host_trigger(&mut callback);
    let request = serde_json::json!({"edits":[
        {"tool":"initialize_modules","arguments":{"ids":["random"]}},
        {"tool":"set_parameter","arguments":{"id":"random","parameter":"random","value":1}},
        {"tool":"set_parameter","arguments":{"id":"random","parameter":"range","value":2}},
        {"tool":"set_parameter","arguments":{"id":"random","parameter":"seed","value":123}}
    ]});
    let preview = session.execute("preview_edits", request.clone()).unwrap();
    let committed = session.execute("apply_edits", request).unwrap();
    assert_eq!(preview["patch"], committed["patch"]);
    shift_register_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let mut rng = 123 ^ 0xA341316C_u32;
    assert_eq!(
        shift_register_host_state(&callback, "random"),
        openpatch_core::SavedModuleState::ShiftRegister {
            values: [0; 8],
            rng
        }
    );
    let saved = session.capture_project_patch(None).unwrap();
    assert_eq!(saved.patch.nodes[1].params["random"], 1.0);
    assert_eq!(saved.patch.nodes[1].params["range"], 2.0);
    assert_eq!(saved.patch.nodes[1].params["seed"], 123.0);
    callback
        .performance
        .trigger("random", ModuleAction::PushPress)
        .unwrap();
    callback
        .performance
        .trigger("random", ModuleAction::PushRelease)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    let first = (rng >> 8) as f32 / 16_777_216.0;
    let mut expected = [0; 8];
    expected[0] = first.to_bits();
    assert_eq!(
        shift_register_host_state(&callback, "random"),
        openpatch_core::SavedModuleState::ShiftRegister {
            values: expected,
            rng
        }
    );
    assert_eq!(
        session.capture_project_patch(None).unwrap().saved_states["random"],
        shift_register_host_state(&callback, "random")
    );
}
