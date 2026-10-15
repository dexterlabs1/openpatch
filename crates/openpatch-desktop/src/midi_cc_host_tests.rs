// Real production callback + Session, without a window or audio/MIDI device.
fn midi_cc_host_patch() -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"cc","kind":"midi_cc","params":{"smooth":0}},
        {"id":"zero","kind":"constant","params":{"value":0}}],
        "output":"cc","output_port":"out1"}))
    .unwrap()
}
fn midi_cc_event(channel: u8, controller: u8, value: u8) -> MidiEvent {
    MidiEvent::ControlChange {
        channel,
        controller,
        value,
    }
}
fn midi_cc_host_render(callback: &mut Callback, frames: usize) {
    let mut output = [0.0_f32; 1025];
    without_heap_activity(|| callback.render(&mut output[..frames], 1));
}
fn midi_cc_host_state(callback: &Callback) -> serde_json::Value {
    callback.performance.snapshot()["modules"]["cc"]["state"].clone()
}
fn midi_cc_host_publish(
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
    // A graph submitted during an existing crossfade is adopted on a later
    // callback. Drive the real exchange until that revision has published.
    for _ in 0..4 {
        midi_cc_host_render(callback, 256);
        while disposal.pop().is_ok() {}
        if callback.graphs.active.revision == session.revision() {
            return;
        }
    }
    panic!("submitted graph was never adopted");
}
#[test]
fn midi_cc_host_learn_qualifies_on_event_sample_and_completion_is_durable_until_commit() {
    let patch = midi_cc_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let mut midi = callback.midi.take_ui().unwrap();
    assert!(midi.send(0, midi_cc_event(0, 7, 42)));
    midi_cc_host_render(&mut callback, 1);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let revision = session.revision();
    session
        .execute("learn_midi_cc", serde_json::json!({"id":"cc","cell":16}))
        .unwrap();
    assert_eq!(
        midi_cc_host_state(&callback)["learning"],
        serde_json::Value::Null,
        "queued intent is not audio state"
    );
    assert!(midi.send(3, midi_cc_event(0, 7, 42))); // Equal committed value cannot learn.
    assert!(midi.send(7, midi_cc_event(0, 7, 99)));
    midi_cc_host_render(&mut callback, 5);
    assert_eq!(midi_cc_host_state(&callback)["learning"]["cell"], 16);
    assert!(callback.performance.midi_cc_learn_ready().is_empty());
    midi_cc_host_render(&mut callback, 4);
    let ready = callback.performance.midi_cc_learn_ready();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].frame, 7);
    assert_eq!(ready[0].completion.cell, 15);
    assert_eq!(ready[0].completion.before[15], 16);
    assert_eq!(ready[0].completion.after[6], -1);
    assert_eq!(ready[0].completion.after[15], 7);
    assert_eq!(
        callback.performance.midi_cc_learn_ready()[0].completion,
        ready[0].completion,
        "peeking must not consume Ready"
    );
    assert!(
        session.capture_project_patch(None).is_err(),
        "capture cannot omit uncommitted Learn"
    );
    assert!(
        callback.performance.cancel_midi_cc_learn("cc").is_err(),
        "cancel cannot erase Ready"
    );
    assert!(callback
        .performance
        .prepare_midi_cc_edit(&["cc".into()])
        .is_err());
    session.reconcile_midi_cc_learn().unwrap();
    assert_eq!(session.revision(), revision + 1);
    assert!(callback.performance.midi_cc_learn_ready().is_empty());
    session.reconcile_midi_cc_learn().unwrap();
    assert_eq!(
        session.revision(),
        revision + 1,
        "exactly one history operation"
    );
    assert_eq!(session.patch().nodes[0].params["cc16"], 7.0);
    assert_eq!(session.patch().nodes[0].params["cc7"], -1.0);
}
#[test]
fn midi_cc_host_mapping_undo_preserves_new_movement_and_graph_adoption_does_not_replay_learn() {
    let patch = midi_cc_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut midi = callback.midi.take_ui().unwrap();
    midi_cc_host_render(&mut callback, 1);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    session
        .execute("learn_midi_cc", serde_json::json!({"id":"cc","cell":1}))
        .unwrap();
    // An unrelated graph edit must preserve the ticket across a real crossfade.
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"zero","parameter":"value","value":1}),
        )
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    assert!(midi.send(2, midi_cc_event(2, 23, 17)));
    midi_cc_host_render(&mut callback, 4);
    let ready = callback.performance.midi_cc_learn_ready();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].frame, 2);
    assert_eq!(ready[0].completion.after[0], 23);
    if let Some(old) = callback.graphs.previous.as_ref() {
        let handle = old.engine.resolve_module_state("cc").unwrap();
        assert!(old.engine.midi_cc_learning(handle).is_none());
    }
    session.reconcile_midi_cc_learn().unwrap();
    midi_cc_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert!(midi.send(callback.frame_clock, midi_cc_event(0, 23, 111)));
    midi_cc_host_render(&mut callback, 1);
    session.execute("undo", serde_json::json!({})).unwrap();
    midi_cc_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let state = midi_cc_host_state(&callback);
    assert_eq!(
        state["assignments"][0],
        1,
        "snapshot={} active={} session={}",
        callback.performance.snapshot(),
        callback.graphs.active.revision,
        session.revision()
    );
    assert_eq!(state["values"][23], 111);
    session.execute("redo", serde_json::json!({})).unwrap();
    midi_cc_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    let state = midi_cc_host_state(&callback);
    assert_eq!(state["assignments"][0], 23);
    assert_eq!(state["values"][23], 111);
    let saved = session.capture_project_patch(None).unwrap();
    assert_eq!(saved.patch.nodes[0].params["cc_value23"], 111.0);
    assert!(callback.performance.midi_cc_learn_ready().is_empty());
}
#[test]
fn midi_cc_host_cancel_panic_bypass_and_reincarnation_cannot_learn_a_late_event() {
    for reason in ["cancel", "panic", "bypass", "remove"] {
        let patch = midi_cc_host_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let mut midi = callback.midi.take_ui().unwrap();
        midi_cc_host_render(&mut callback, 1);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        session
            .execute("learn_midi_cc", serde_json::json!({"id":"cc","cell":1}))
            .unwrap();
        midi_cc_host_render(&mut callback, 1);
        assert_eq!(midi_cc_host_state(&callback)["learning"]["cell"], 1);
        match reason {
            "cancel" => {
                session
                    .execute("cancel_midi_cc_learn", serde_json::json!({"id":"cc"}))
                    .unwrap();
            }
            "panic" => {
                callback.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
            }
            "bypass" => {
                session
                    .execute("set_bypass", serde_json::json!({"id":"cc","bypassed":true}))
                    .unwrap();
                midi_cc_host_publish(&mut callback, &mut updates, &mut disposal, &session);
            }
            "remove" => {
                session
                    .execute("remove_module", serde_json::json!({"id":"cc"}))
                    .unwrap();
                midi_cc_host_publish(&mut callback, &mut updates, &mut disposal, &session);
                session
                    .execute(
                        "add_module",
                        serde_json::json!({"id":"cc","kind":"midi_cc","params":{"smooth":0}}),
                    )
                    .unwrap();
                midi_cc_host_publish(&mut callback, &mut updates, &mut disposal, &session);
            }
            _ => unreachable!(),
        }
        assert!(midi.send(callback.frame_clock, midi_cc_event(0, 71, 127)));
        midi_cc_host_render(&mut callback, 1);
        assert!(
            callback.performance.midi_cc_learn_ready().is_empty(),
            "{reason}"
        );
        assert_eq!(
            midi_cc_host_state(&callback)["assignments"][0],
            1,
            "{reason}"
        );
        assert_eq!(
            midi_cc_host_state(&callback)["learning"],
            serde_json::Value::Null,
            "{reason}"
        );
    }
}
#[test]
fn midi_cc_host_two_modules_finish_in_deterministic_order_and_headless_worker_records_history() {
    let mut patch = midi_cc_host_patch();
    let mut other = patch.nodes[0].clone();
    other.id = "cc2".into();
    patch.nodes.push(other);
    let (mut callback, _, _) = sequence_callback(&patch);
    let mut midi = callback.midi.take_ui().unwrap();
    midi_cc_host_render(&mut callback, 1);
    let workspace = tempfile::tempdir().unwrap();
    let shared = openpatch::control::Session::new(workspace.path())
        .unwrap()
        .into_shared();
    openpatch::control::execute_shared(
        &shared,
        "replace_patch",
        serde_json::json!({"patch":patch}),
    )
    .unwrap();
    shared
        .lock()
        .unwrap()
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    for id in ["cc2", "cc"] {
        openpatch::control::execute_shared(
            &shared,
            "learn_midi_cc",
            serde_json::json!({"id":id,"cell":1}),
        )
        .unwrap();
    }
    assert!(midi.send(4, midi_cc_event(0, 67, 88)));
    midi_cc_host_render(&mut callback, 8);
    // The GUI never runs; the Session's single weak worker owns reconciliation.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let s = shared.lock().unwrap();
        if s.patch()
            .nodes
            .iter()
            .filter(|n| n.kind == "midi_cc")
            .all(|n| n.params.get("cc1") == Some(&67.0))
        {
            assert_eq!(s.revision(), 3);
            break;
        }
        drop(s);
        assert!(
            std::time::Instant::now() < deadline,
            "headless Learn worker timed out"
        );
        std::thread::yield_now();
    }
    assert!(callback.performance.midi_cc_learn_ready().is_empty());
}
#[test]
fn midi_cc_host_ready_survives_panic_overflow_and_close_without_callback_allocation() {
    let patch = midi_cc_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let mut midi = callback.midi.take_ui().unwrap();
    callback.performance.learn_midi_cc("cc", 0).unwrap();
    assert!(midi.send(0, midi_cc_event(0, 71, 127)));
    midi_cc_host_render(&mut callback, 1);
    let ready = callback.performance.midi_cc_learn_ready();
    assert_eq!(ready.len(), 1);
    callback.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
    midi_cc_host_render(&mut callback, 1);
    assert_eq!(
        callback.performance.midi_cc_learn_ready()[0].completion,
        ready[0].completion
    );
    // Same cleanup route is taken after a bounded MIDI source overflow.
    for i in 0..(MIDI_QUEUE_SIZE + 1) {
        midi.send(callback.frame_clock, midi_cc_event(0, 72, (i % 128) as u8));
    }
    midi_cc_host_render(&mut callback, 1);
    assert_eq!(
        callback.performance.midi_cc_learn_ready()[0].completion,
        ready[0].completion
    );
    callback.performance.close();
    midi_cc_host_render(&mut callback, 1);
    assert_eq!(
        callback.performance.midi_cc_learn_ready()[0].completion,
        ready[0].completion
    );
}
#[test]
fn midi_cc_host_reset_also_cancels_learn_queued_for_a_future_graph() {
    let mut old = midi_cc_host_patch();
    old.nodes.remove(0);
    old.output = Some("zero".into());
    old.output_port = "out".into();
    let (mut callback, mut updates, _disposal) = sequence_callback(&old);
    let mut midi = callback.midi.take_ui().unwrap();
    midi_cc_host_render(&mut callback, 1);
    let added = midi_cc_host_patch();
    callback.performance.sync_patch(&added, 2);
    callback.performance.learn_midi_cc("cc", 0).unwrap();
    // Registry/agent is ahead of the graph exchange when hardware disconnects.
    callback.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
    midi_cc_host_render(&mut callback, 1);
    assert!(updates.push(sequence_graph(&callback, &added, 2)).is_ok());
    midi_cc_host_render(&mut callback, 256);
    assert!(midi.send(callback.frame_clock, midi_cc_event(0, 71, 127)));
    midi_cc_host_render(&mut callback, 1);
    assert!(callback.performance.midi_cc_learn_ready().is_empty());
    assert_eq!(midi_cc_host_state(&callback)["assignments"][0], 1);
    assert_eq!(
        midi_cc_host_state(&callback)["learning"],
        serde_json::Value::Null
    );
}
