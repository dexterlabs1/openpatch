// Real callback execution permission, separate from transport/speaker mute.
use openpatch::project_document::ExecutionDisposition;

fn import_execution_scope_index(graph: &PreparedGraph) -> (usize, usize) {
    let handle = graph.engine.resolve_module_scope("scope").unwrap();
    let scope = graph.engine.module_scope(handle).unwrap();
    (scope.buffer_index, scope.sample_index as usize)
}

#[test]
fn import_execution_suspension_preserves_delay_source_and_scope_through_two_revisions() {
    let mut patch = stock_delay_callback_patch(0.0, true);
    patch.nodes.push(
        serde_json::from_value(serde_json::json!({
            "id":"scope","kind":"scope","params":{"time":0.005}
        }))
        .unwrap(),
    );
    patch.cables.push(
        serde_json::from_value(serde_json::json!({
            "id":"delay_scope","from":"delay","output":"mix","to":"scope","input":"x"
        }))
        .unwrap(),
    );
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut warm = [0.0_f32; 4099];
    without_heap_activity(|| callback.render(&mut warm, 1));
    assert!(warm.iter().any(|sample| sample.abs() > 0.001));
    assert!(callback.performance.scope_frame("scope").is_some());
    let scope_index = import_execution_scope_index(&callback.graphs.active);
    let tracker = stock_delay_tracker(&callback.graphs.active);
    let clock_light = stock_delay_clock_light(&callback.graphs.active);
    let mut reference = patch.compile().unwrap();
    reference.adopt_state_from(&callback.graphs.active.engine);
    let mut expected = [[0.0; 2]; 513];
    reference.process_audio_block(&mut expected);

    for revision in [2, 3] {
        if revision == 2 {
            callback
                .performance
                .request_document_execution(revision, ExecutionDisposition::SuspendedUnresolved);
        }
        // The second edit deliberately has no new execution request. It must
        // inherit suspension instead of silently choosing partial playback.
        callback.performance.sync_patch(&patch, revision);
        assert!(!callback.performance.execution_available());
        assert!(callback.performance.scope_frame("scope").is_none());
        assert!(callback.performance.module_lights("delay").is_none());
        let snapshot = callback.performance.snapshot();
        assert_eq!(snapshot["available"], false);
        assert_eq!(snapshot["coherent"], false);
        assert!(snapshot["unavailable_reason"].is_string());
        updates
            .push(sequence_graph(&callback, &patch, revision))
            .ok()
            .unwrap();
        let mut silent = [99.0_f32; 2049];
        without_heap_activity(|| callback.render(&mut silent, 1));
        assert_eq!(silent, [0.0; 2049]);
        assert_eq!(
            import_execution_scope_index(&callback.graphs.active),
            scope_index
        );
        assert_eq!(stock_delay_tracker(&callback.graphs.active), tracker);
        assert_eq!(
            stock_delay_clock_light(&callback.graphs.active),
            clock_light
        );
        assert!(callback.graphs.previous.is_none());
        assert!(disposal.pop().is_ok(), "suspended donor must retire");
        assert!(callback.performance.scope_identity("scope").is_some());
    }
    callback
        .performance
        .request_document_execution(4, ExecutionDisposition::RunnablePartial);
    callback.performance.sync_patch(&patch, 4);
    updates
        .push(sequence_graph(&callback, &patch, 4))
        .ok()
        .unwrap();
    assert!(!callback.performance.execution_available());
    let mut actual = [0.0_f32; 513];
    without_heap_activity(|| callback.render(&mut actual, 1));
    assert!(callback.performance.execution_available());
    assert_eq!(callback.performance.snapshot()["coherent"], true);
    assert!(callback.graphs.previous.is_none());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!((actual - expected[0].clamp(-1.0, 1.0)).abs() < 1e-6);
    }
    let reference_scope = reference
        .module_scope(reference.resolve_module_scope("scope").unwrap())
        .unwrap();
    assert_eq!(
        import_execution_scope_index(&callback.graphs.active),
        (
            reference_scope.buffer_index,
            reference_scope.sample_index as usize
        )
    );
}

#[test]
fn import_execution_stale_runnable_plan_cannot_acknowledge_newer_suspension() {
    let patch = sequence_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    callback
        .performance
        .trigger("seq", ModuleAction::Clock)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let frozen = callback.performance.sequence_state("seq").unwrap();
    callback
        .performance
        .request_document_execution(2, ExecutionDisposition::RunnablePartial);
    callback.performance.sync_patch(&patch, 2);
    let stale = sequence_graph(&callback, &patch, 2);
    callback
        .performance
        .request_document_execution(3, ExecutionDisposition::SuspendedUnresolved);
    callback.performance.sync_patch(&patch, 3);
    updates.push(stale).ok().unwrap();
    let mut output = [99.0_f32; 513];
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(output, [0.0; 513]);
    assert!(!callback.performance.execution_available());
    assert!(callback.performance.sequence_state("seq").is_none());
    assert!(callback.graphs.previous.is_none());
    assert!(disposal.pop().is_ok());
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_telemetry("seq")
        .unwrap();
    assert_eq!(
        callback
            .graphs
            .active
            .engine
            .module_telemetry(handle)
            .unwrap(),
        frozen
    );
    updates
        .push(sequence_graph(&callback, &patch, 3))
        .ok()
        .unwrap();
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(output, [0.0; 513]);
    assert!(disposal.pop().is_ok());
    // Actions queued while suspended cannot become new notes/actions on repair.
    callback
        .performance
        .trigger("seq", ModuleAction::Clock)
        .unwrap();
    callback
        .performance
        .request_document_execution(4, ExecutionDisposition::RunnableExact);
    callback.performance.sync_patch(&patch, 4);
    updates
        .push(sequence_graph(&callback, &patch, 4))
        .ok()
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(callback.performance.sequence_state("seq").unwrap(), frozen);
    assert_eq!(callback.performance.snapshot()["pending"], 0);
}

#[test]
fn import_execution_cancels_manual_holds_and_future_midi_without_replay() {
    let patch = manual_patch("push");
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    callback
        .performance
        .trigger("manual", ModuleAction::PushPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(
        callback
            .performance
            .module_control_state("manual")
            .unwrap()
            .pressed
    );
    callback
        .performance
        .request_document_execution(2, ExecutionDisposition::SuspendedUnresolved);
    callback.performance.sync_patch(&patch, 2);
    updates
        .push(sequence_graph(&callback, &patch, 2))
        .ok()
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    assert!(disposal.pop().is_ok());
    callback
        .performance
        .trigger("manual", ModuleAction::PushRelease)
        .unwrap();
    callback
        .performance
        .trigger("manual", ModuleAction::PushPress)
        .unwrap();
    callback
        .performance
        .request_document_execution(3, ExecutionDisposition::RunnablePartial);
    callback.performance.sync_patch(&patch, 3);
    updates
        .push(sequence_graph(&callback, &patch, 3))
        .ok()
        .unwrap();
    let mut output = [99.0_f32; 64];
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(output, [0.0; 64]);
    assert!(
        !callback
            .performance
            .module_control_state("manual")
            .unwrap()
            .pressed
    );
    assert_eq!(callback.performance.snapshot()["pending"], 0);

    let patch = midi_patch();
    let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
    let mut midi = callback.midi.take_ui().unwrap();
    assert!(midi.send(0, note_on(60)));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    callback
        .performance
        .request_document_execution(2, ExecutionDisposition::SuspendedUnresolved);
    callback.performance.sync_patch(&patch, 2);
    assert!(midi.send(1000, note_on(62)));
    // Even if suspend and resume arrive between callbacks, old queued input is
    // fenced by the execution cancellation generation.
    callback
        .performance
        .request_document_execution(3, ExecutionDisposition::RunnablePartial);
    callback.performance.sync_patch(&patch, 3);
    updates
        .push(sequence_graph(&callback, &patch, 3))
        .ok()
        .unwrap();
    let mut output = [99.0_f32; 1200];
    without_heap_activity(|| callback.render(&mut output, 1));
    assert_eq!(output, [0.0; 1200]);
}

#[test]
fn import_execution_default_exact_preserves_native_attach_and_mute_processing() {
    let patch = rack_scope_patch(false);
    let (mut callback, _, _disposal) = sequence_callback(&patch);
    callback
        .performance
        .request_document_execution(1, ExecutionDisposition::RunnableExact);
    callback.shared.playing.store(false, Ordering::Relaxed);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    assert!(callback.performance.execution_available());
    assert_eq!(
        callback
            .performance
            .scope_frame("scope")
            .unwrap()
            .snapshot
            .buffer_index,
        64
    );
}

#[test]
fn import_execution_pending_runnable_edit_preserves_held_midi_and_new_revision_actions() {
    let mut patch = midi_patch();
    patch.nodes.push(
        serde_json::from_value(serde_json::json!({
            "id":"manual","kind":"push","params":{"holding":0}
        }))
        .unwrap(),
    );
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut midi = callback.midi.take_ui().unwrap();
    callback
        .performance
        .request_document_execution(1, ExecutionDisposition::RunnablePartial);
    updates
        .push(sequence_graph(&callback, &patch, 1))
        .ok()
        .unwrap();
    assert!(midi.send(0, note_on(60)));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let midi_epoch = callback.shared.midi_epoch.load(Ordering::SeqCst);

    // Exercise partial edits and the latched execution fence after returning to
    // native/exact execution. A callback can arrive before its new graph does.
    for (revision, disposition, action, pressed) in [
        (
            2,
            ExecutionDisposition::RunnablePartial,
            ModuleAction::PushPress,
            true,
        ),
        (
            3,
            ExecutionDisposition::RunnableExact,
            ModuleAction::PushRelease,
            false,
        ),
        (
            4,
            ExecutionDisposition::RunnableExact,
            ModuleAction::PushPress,
            true,
        ),
    ] {
        if revision != 4 {
            callback
                .performance
                .request_document_execution(revision, disposition);
        }
        callback.performance.sync_patch(&patch, revision);
        callback.performance.trigger("manual", action).unwrap();
        let mut output = [99.0_f32; 64];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 64]);
        assert_eq!(callback.performance.snapshot()["pending"], 1);
        assert!(!callback.performance.execution_available());
        assert_eq!(
            callback.shared.midi_epoch.load(Ordering::SeqCst),
            midi_epoch
        );
        updates
            .push(sequence_graph(&callback, &patch, revision))
            .ok()
            .unwrap();
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(
            output, [1.0; 64],
            "held MIDI note lost at revision {revision}"
        );
        assert_eq!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed,
            pressed
        );
        assert_eq!(callback.performance.snapshot()["pending"], 0);
        assert!(callback.performance.execution_available());
        assert_eq!(
            callback.shared.midi_epoch.load(Ordering::SeqCst),
            midi_epoch
        );
        assert!(disposal.pop().is_ok());
    }
}
