// Included in audio::tests: exercise the real headless callback and performance
// registry without creating a device or replacing their production code paths.
fn adsr_eg_host_patch(width: u8, gate: f32) -> Patch {
    let mut params = serde_json::json!({"channels":width});
    for channel in 1..=16 {
        params[format!("v{channel}")] = serde_json::json!(if channel == 1 || channel == width {
            gate
        } else {
            0.0
        });
    }
    let mut patch: Patch =
        serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"gate","kind":"poly_constant","params":params},
                 {"id":"envelope","kind":"adsr_eg"}],
        "cables":[{"from":"gate","to":"envelope","input":"gate"}],
        "output":"envelope"}))
        .unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}
fn adsr_eg_host_render(callback: &mut Callback, frames: usize) {
    let mut output = [0.0_f32; 1025];
    without_heap_activity(|| callback.render(&mut output[..frames], 1));
    assert!(output[..frames].iter().all(|v| v.is_finite()));
}
fn adsr_eg_host_token(callback: &Callback) -> u64 {
    callback
        .graphs
        .active
        .modules
        .bindings
        .iter()
        .flatten()
        .next()
        .unwrap()
        .token
}
fn adsr_eg_host_held(callback: &Callback) -> bool {
    callback
        .performance
        .module_control_state("envelope")
        .unwrap()
        .pressed
}
fn adsr_eg_host_publish(
    callback: &mut Callback,
    updates: &mut Producer<PreparedGraph>,
    disposal: &mut Consumer<PreparedGraph>,
    patch: &Patch,
    revision: u64,
    frames: usize,
) {
    callback.performance.sync_patch(patch, revision);
    assert!(updates
        .push(sequence_graph(callback, patch, revision))
        .is_ok());
    adsr_eg_host_render(callback, frames);
    while disposal.pop().is_ok() {}
}

#[test]
fn adsr_eg_host_real_poly_gate_lamps_controls_and_revision_publication() {
    let mut patch = adsr_eg_host_patch(5, 10.0);
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    assert!(callback.performance.module_lights("envelope").is_none());
    adsr_eg_host_render(&mut callback, 127);
    let before = callback.performance.module_lights("envelope").unwrap();
    assert_eq!(before.count, 5);
    assert_eq!(before.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    adsr_eg_host_render(&mut callback, 1);
    let published = callback.performance.snapshot();
    assert_eq!(published["coherent"], true);
    assert_eq!(published["modules"]["envelope"]["kind"], "adsr_eg");
    assert_eq!(published["modules"]["envelope"]["available"], true);
    assert_eq!(published["modules"]["envelope"]["revision"], 1);
    assert_eq!(published["modules"]["envelope"]["frame"], 128);
    assert_eq!(
        published["modules"]["envelope"]["state"],
        serde_json::Value::Null
    );
    let lights = callback.performance.module_lights("envelope").unwrap();
    assert_eq!(lights.input_channels, Some(5));
    assert_eq!(lights.brightness[0], [1.0, 0.0]);
    assert_eq!(lights.brightness[4], [1.0, 0.0]);
    assert!(!lights.manual && !adsr_eg_host_held(&callback));
    let mut frames = [openpatch_core::PortFrame::default()];
    without_heap_activity(|| {
        callback
            .graphs
            .active
            .engine
            .process_poly_block(&mut frames)
    });
    assert_eq!(frames[0].channels, 5);
    assert!(frames[0].values[0] > 0.0 && frames[0].values[4] > 0.0);
    assert_eq!(frames[0].values[1..4], [0.0; 3]);
    assert_eq!(frames[0].values[5..], [0.0; 11]);

    patch.nodes[1].params.insert("sustain".into(), 0.25);
    callback.performance.sync_patch(&patch, 2);
    assert!(callback.performance.module_lights("envelope").is_none());
    let ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 1);
    assert_eq!(callback.performance.snapshot()["pending"], 1);
    assert!(callback
        .performance
        .module_control_state("envelope")
        .is_none());
    adsr_eg_host_publish(&mut callback, &mut updates, &mut disposal, &patch, 2, 128);
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["envelope"]["revision"], 2);
    assert_eq!(
        published["modules"]["envelope"]["frame"],
        callback.frame_clock
    );
    assert_eq!(published["pending"], 0);
    assert!(adsr_eg_host_held(&callback));
    assert!(
        callback
            .performance
            .module_lights("envelope")
            .unwrap()
            .manual
    );
    drop(ticket);
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
    assert!(
        !callback
            .performance
            .module_lights("envelope")
            .unwrap()
            .manual
    );
}

#[test]
fn adsr_eg_host_ticket_enqueues_one_press_and_next_frame_drop_releases_without_allocating() {
    let patch = adsr_eg_host_patch(16, 0.0);
    let (mut callback, _, _) = sequence_callback(&patch);
    assert!(callback
        .performance
        .press_gate_with_ticket("missing")
        .is_err());
    assert_eq!(callback.performance.snapshot()["queued"], 0);
    let ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    let queued = callback.performance.snapshot();
    assert_eq!(queued["queued"], 1);
    assert_eq!(queued["applied"], 0);
    assert_eq!(queued["pending"], 1);
    // Simulate the UI retaining its fast-click ticket until a later UI frame.
    adsr_eg_host_render(&mut callback, 128);
    assert!(adsr_eg_host_held(&callback));
    let lights = callback.performance.module_lights("envelope").unwrap();
    assert!(lights.manual);
    assert_eq!(lights.input_channels, Some(16));
    assert_eq!(lights.brightness[4], [1.0, 0.0]);
    drop(ticket);
    assert_eq!(callback.performance.snapshot()["queued"], 2);
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
    let status = callback.performance.snapshot();
    assert_eq!(status["applied"], 2);
    assert_eq!(status["pending"], 0);
    let mut tail = [openpatch_core::PortFrame::default()];
    without_heap_activity(|| callback.graphs.active.engine.process_poly_block(&mut tail));
    assert_eq!(tail[0].channels, 16);
    assert!(tail[0].values.iter().all(|v| *v > 0.0));
}

#[test]
fn adsr_eg_host_simultaneous_ticket_press_release_samples_zero_without_a_ghost_envelope() {
    let patch = adsr_eg_host_patch(16, 0.0);
    let (mut callback, _, _) = sequence_callback(&patch);
    let ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    drop(ticket);
    adsr_eg_host_render(&mut callback, 256);
    assert!(!adsr_eg_host_held(&callback));
    let status = callback.performance.snapshot();
    assert_eq!(status["applied"], 2);
    assert_eq!(status["pending"], 0);
    let lights = callback.performance.module_lights("envelope").unwrap();
    assert!(!lights.manual);
    assert_eq!(lights.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    let mut frames = [openpatch_core::PortFrame::default(); 16];
    without_heap_activity(|| {
        callback
            .graphs
            .active
            .engine
            .process_poly_block(&mut frames)
    });
    assert!(frames
        .iter()
        .all(|f| f.channels == 16 && f.values == [0.0; 16]));
}

#[test]
fn adsr_eg_host_removed_and_readded_same_id_rejects_old_ticket_without_touching_new_press() {
    let patch = adsr_eg_host_patch(1, 0.0);
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let old_ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 32);
    let old_token = adsr_eg_host_token(&callback);
    let mut absent = patch.clone();
    absent.nodes.clear();
    absent.cables.clear();
    absent.output = None;
    adsr_eg_host_publish(&mut callback, &mut updates, &mut disposal, &absent, 2, 256);
    callback.performance.sync_patch(&patch, 3);
    let new_ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_publish(&mut callback, &mut updates, &mut disposal, &patch, 3, 256);
    assert_ne!(old_token, adsr_eg_host_token(&callback));
    assert!(adsr_eg_host_held(&callback));
    let before = callback.performance.snapshot();
    drop(old_ticket);
    let after = callback.performance.snapshot();
    for counter in [
        "queued",
        "backpressure",
        "emergency_release_requests",
        "rejected",
    ] {
        assert_eq!(before[counter], after[counter], "{counter}");
    }
    adsr_eg_host_render(&mut callback, 1);
    assert!(adsr_eg_host_held(&callback));
    drop(new_ticket);
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
}

#[test]
fn adsr_eg_host_initialize_changes_action_epoch_and_obsolete_drop_does_not_release_new_press() {
    let patch = adsr_eg_host_patch(1, 0.0);
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let old_ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 128);
    let token = adsr_eg_host_token(&callback);
    callback.performance.sync_patch(&patch, 2);
    callback
        .performance
        .initialize_modules(&["envelope".into()]);
    adsr_eg_host_publish(&mut callback, &mut updates, &mut disposal, &patch, 2, 256);
    assert_eq!(token, adsr_eg_host_token(&callback));
    assert!(!adsr_eg_host_held(&callback));
    let new_ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 1);
    assert!(adsr_eg_host_held(&callback));
    let before = callback.performance.snapshot();
    drop(old_ticket);
    let after = callback.performance.snapshot();
    assert_eq!(before["queued"], after["queued"]);
    assert_eq!(
        before["emergency_release_requests"],
        after["emergency_release_requests"]
    );
    adsr_eg_host_render(&mut callback, 1);
    assert!(adsr_eg_host_held(&callback));
    drop(new_ticket);
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
}

#[test]
fn adsr_eg_host_replacement_backend_with_equal_numeric_token_keeps_release_on_original_backend() {
    let patch = adsr_eg_host_patch(1, 0.0);
    let (mut original, _, _) = sequence_callback(&patch);
    let (mut replacement, _, _) = sequence_callback(&patch);
    assert_eq!(
        adsr_eg_host_token(&original),
        adsr_eg_host_token(&replacement)
    );
    let old_ticket = original
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    let new_ticket = replacement
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut original, 1);
    adsr_eg_host_render(&mut replacement, 1);
    assert!(adsr_eg_host_held(&original) && adsr_eg_host_held(&replacement));
    drop(old_ticket);
    adsr_eg_host_render(&mut original, 1);
    adsr_eg_host_render(&mut replacement, 1);
    assert!(!adsr_eg_host_held(&original));
    assert!(adsr_eg_host_held(&replacement));
    assert_eq!(replacement.performance.snapshot()["queued"], 1);
    drop(new_ticket);
    adsr_eg_host_render(&mut replacement, 1);
    assert!(!adsr_eg_host_held(&replacement));
}

#[test]
fn adsr_eg_host_ticket_drop_on_full_queue_emergency_releases_valid_identity_and_cancels_stale_presses(
) {
    let patch = adsr_eg_host_patch(16, 0.0);
    let (mut callback, _, _) = sequence_callback(&patch);
    let ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 128);
    for _ in 0..EVENT_CAPACITY {
        callback
            .performance
            .trigger("envelope", ModuleAction::PushPress)
            .unwrap();
    }
    assert!(callback
        .performance
        .press_gate_with_ticket("envelope")
        .is_err());
    assert_eq!(callback.performance.snapshot()["pending"], EVENT_CAPACITY);
    drop(ticket);
    let requested = callback.performance.snapshot();
    assert_eq!(requested["emergency_release_requests"], 1);
    assert_eq!(requested["backpressure"], 2); // failed press and guarded safety release
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
    let applied = callback.performance.snapshot();
    assert_eq!(applied["emergency_release_applied"], 1);
    assert_eq!(applied["dropped"], EVENT_CAPACITY);
    assert_eq!(applied["applied"], 1);
    assert_eq!(applied["pending"], 0);
    let next = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 1);
    assert!(adsr_eg_host_held(&callback));
    drop(next);
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
}

#[test]
fn adsr_eg_host_obsolete_ticket_drop_on_full_queue_never_requests_emergency_release() {
    let patch = adsr_eg_host_patch(16, 0.0);
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let old_ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 128);
    callback.performance.sync_patch(&patch, 2);
    callback
        .performance
        .initialize_modules(&["envelope".into()]);
    adsr_eg_host_publish(&mut callback, &mut updates, &mut disposal, &patch, 2, 256);
    let new_ticket = callback
        .performance
        .press_gate_with_ticket("envelope")
        .unwrap();
    adsr_eg_host_render(&mut callback, 1);
    for _ in 0..EVENT_CAPACITY {
        callback
            .performance
            .trigger("envelope", ModuleAction::PushPress)
            .unwrap();
    }
    let before = callback.performance.snapshot();
    drop(old_ticket);
    let after = callback.performance.snapshot();
    for counter in [
        "queued",
        "pending",
        "backpressure",
        "emergency_release_requests",
        "rejected",
    ] {
        assert_eq!(before[counter], after[counter], "{counter}");
    }
    adsr_eg_host_render(&mut callback, 8);
    assert!(adsr_eg_host_held(&callback));
    assert_eq!(callback.performance.snapshot()["pending"], 0);
    assert_eq!(
        callback.performance.snapshot()["emergency_release_requests"],
        before["emergency_release_requests"]
    );
    drop(new_ticket);
    adsr_eg_host_render(&mut callback, 1);
    assert!(!adsr_eg_host_held(&callback));
}
