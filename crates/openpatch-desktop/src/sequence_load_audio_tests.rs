fn sequence_load_patch() -> Patch {
    let mut patch = live_parameter_audio_patch();
    patch.nodes.insert(0, serde_json::from_value(serde_json::json!({
        "id":"automation","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":{"data":{"patterns":[{"name":"A","length":4}],
            "lanes":[{"moduleId":"42","paramId":0,"points":[{"bar":0,"value":0.25}]}]},
            "targets":[{"source_module_id":"42","source_param_id":0,"module":"vca","parameter":"level"}]}
    })).unwrap());
    patch
}

fn sync_sequence_load(callback: &Callback, patch: &Patch, revision: u64) {
    callback.performance.sync_patch_with_intents(
        patch,
        revision,
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeSet::from(["automation".into()]),
    );
}

fn render_sequence_lane(callback: &mut Callback, expected: f32) {
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    assert_live_parameter(callback, "vca", "level", expected);
}

#[test]
fn sequence_load_equal_data_rearms_lane_and_unrelated_edits_preserve_manual_override() {
    let mut patch = sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    render_sequence_lane(&mut callback, 0.25);
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    patch.nodes[0].params.insert("bpm".into(), 128.0);
    callback.performance.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    while retired.pop().is_ok() {}
    sync_sequence_load(&callback, &patch, 3);
    // Ordinary attachment/retry at the same revision must preserve the load.
    callback.performance.sync_patch(&patch, 3);
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    callback.performance.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.6);
}

#[test]
fn sequence_load_prepared_graphs_freeze_epochs_and_skipped_loads_survive() {
    let patch = sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    render_sequence_lane(&mut callback, 0.25);
    callback.performance.sync_patch(&patch, 2);
    let frozen_unrelated = sequence_graph(&callback, &patch, 2);
    sync_sequence_load(&callback, &patch, 3);
    let loaded = sequence_graph(&callback, &patch, 3);
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    assert!(updates.push(frozen_unrelated).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    while retired.pop().is_ok() {}
    assert!(updates.push(loaded).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}

    sync_sequence_load(&callback, &patch, 4);
    let earlier_load = sequence_graph(&callback, &patch, 4);
    sync_sequence_load(&callback, &patch, 5);
    let later_load = sequence_graph(&callback, &patch, 5);
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    assert!(updates.push(earlier_load).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    assert!(updates.push(later_load).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}

    sync_sequence_load(&callback, &patch, 6);
    // No revision-6 graph ever reaches the callback.
    callback.performance.sync_patch(&patch, 7);
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    assert!(updates.push(sequence_graph(&callback, &patch, 7)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    callback.performance.sync_patch(&patch, 7);
    assert!(updates.push(sequence_graph(&callback, &patch, 7)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
}

#[test]
fn sequence_load_late_frozen_graphs_never_replay_consumed_loads() {
    for older_has_load in [false, true] {
        let patch = sequence_load_patch();
        let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
        render_sequence_lane(&mut callback, 0.25);
        if older_has_load {
            sync_sequence_load(&callback, &patch, 2);
        } else {
            callback.performance.sync_patch(&patch, 2);
        }
        let older = sequence_graph(&callback, &patch, 2);
        sync_sequence_load(&callback, &patch, 3);
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        render_sequence_lane(&mut callback, 0.25);
        while retired.pop().is_ok() {}

        set_live_parameter(&mut callback, "vca", "level", 0.75);
        // This already-prepared plan carries epoch zero or an earlier explicit
        // load. Its request must not replace the live applied high water mark.
        assert!(updates.push(older).is_ok());
        render_sequence_lane(&mut callback, 0.75);
        while retired.pop().is_ok() {}
        callback.performance.sync_patch(&patch, 4);
        assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
        render_sequence_lane(&mut callback, 0.75);
        while retired.pop().is_ok() {}

        sync_sequence_load(&callback, &patch, 5);
        assert!(updates.push(sequence_graph(&callback, &patch, 5)).is_ok());
        render_sequence_lane(&mut callback, 0.25);
    }
}

#[test]
fn sequence_load_cross_rate_adoption_retains_cache_and_consumes_explicit_event_once() {
    for load in [false, true] {
        let patch = sequence_load_patch();
        let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
        render_sequence_lane(&mut callback, 0.25);
        set_live_parameter(&mut callback, "vca", "level", 0.75);
        if load {
            sync_sequence_load(&callback, &patch, 2);
        } else {
            callback.performance.sync_patch(&patch, 2);
        }
        let mut graph = prepare_graph(&patch, 96_000, None, 2, true).unwrap();
        graph.modules = callback.performance.plan(&patch, &graph.engine, 2).unwrap();
        graph.revision = 2;
        assert!(updates.push(graph).is_ok());
        render_sequence_lane(&mut callback, if load { 0.25 } else { 0.75 });
        while retired.pop().is_ok() {}
        set_live_parameter(&mut callback, "vca", "level", 0.6);
        callback.performance.sync_patch(&patch, 3);
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        render_sequence_lane(&mut callback, 0.6);
    }
}

#[test]
fn sequence_load_while_suspended_rearms_lane_on_resume_without_replaying() {
    use openpatch::project_document::ExecutionDisposition;
    let patch = sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    render_sequence_lane(&mut callback, 0.25);
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    callback
        .performance
        .request_document_execution(2, ExecutionDisposition::SuspendedUnresolved);
    sync_sequence_load(&callback, &patch, 2);
    for revision in [2, 3] {
        callback.performance.sync_patch(&patch, revision);
        assert!(updates
            .push(sequence_graph(&callback, &patch, revision))
            .is_ok());
        let mut silence = [1.0_f32; 512];
        without_heap_activity(|| callback.render(&mut silence, 1));
        assert!(silence.iter().all(|value| *value == 0.0));
        assert_live_parameter(&callback, "vca", "level", 0.75);
        while retired.pop().is_ok() {}
    }
    callback
        .performance
        .request_document_execution(4, ExecutionDisposition::RunnablePartial);
    callback.performance.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    callback.performance.sync_patch(&patch, 5);
    assert!(updates.push(sequence_graph(&callback, &patch, 5)).is_ok());
    render_sequence_lane(&mut callback, 0.6);
}

fn sequence_load_selection_patch(selected: &str) -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"nodes":[{
        "id":"automation","kind":"rackmcp_sequencer","params":{"running":1,"bpm":300},
        "rackmcp_sequence":{"data":{"selectedPattern":selected,"patterns":[
            {"name":"A","length":4,"stepsPerBeat":8,"tracks":[{"steps":[
                {"step":0,"notes":[0]},{"step":1,"notes":[12]},
                {"step":2,"notes":[24]},{"step":3,"notes":[36]}]}]},
            {"name":"B","length":4,"stepsPerBeat":8,"tracks":[{"steps":[
                {"step":0,"notes":[24]},{"step":1,"notes":[36]},
                {"step":2,"notes":[48]},{"step":3,"notes":[60]}]}]}
        ]}}}],"output":"automation","output_port":"pitch1"}))
    .unwrap()
}

#[test]
fn sequence_load_fresh_and_host_excluded_nonzero_selection_keep_first_advance_request() {
    for host_reset in [false, true] {
        let patch = sequence_load_selection_patch("B");
        let (mut callback, mut updates, _retired) = sequence_callback(&patch);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 3000], 1));
        if !host_reset {
            callback.performance.retire_modules(&["automation".into()]);
        }
        sync_sequence_load(&callback, &patch, 2);
        let mut graph = sequence_graph(&callback, &patch, 2);
        if host_reset {
            let mut delta = HostLifecycleDelta::default();
            delta.modules.entry("automation".into()).or_default().reset = true;
            graph.host_lifecycle = PreparedHostLifecycle::prepare(&graph.engine, delta).unwrap();
        }
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let mut pitch = [0.0_f32; 1200];
        without_heap_activity(|| callback.graphs.active.engine.process_block(&mut pitch));
        // Source first load starts A at step zero, then selects B at the very
        // first advance. A second load would replace now=true with wrap=false.
        assert_eq!(pitch[0], 0.0);
        assert_eq!(pitch[1199], 3.0);
    }
}

#[test]
fn sequence_load_equal_data_preserves_current_playhead() {
    let patch = sequence_load_selection_patch("A");
    let (mut callback, mut updates, _retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1500], 1));
    sync_sequence_load(&callback, &patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let mut pitch = [0.0_f32; 1];
    without_heap_activity(|| callback.graphs.active.engine.process_block(&mut pitch));
    assert_eq!(pitch[0], 1.0);
}

#[test]
fn sequence_load_shared_equal_preset_undo_redo_rearm_constant_lane() {
    let patch = sequence_load_patch();
    let preset = openpatch::presets::capture(&patch.nodes[0], None).unwrap();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    render_sequence_lane(&mut callback, 0.25);
    for (tool, arguments) in [
        (
            "apply_module_preset",
            serde_json::json!({"id":"automation","preset":preset}),
        ),
        ("undo", serde_json::json!({})),
        ("redo", serde_json::json!({})),
    ] {
        set_live_parameter(&mut callback, "vca", "level", 0.75);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_live_parameter(&callback, "vca", "level", 0.75);
        session.execute(tool, arguments).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        render_sequence_lane(&mut callback, 0.25);
        while retired.pop().is_ok() {}
    }
}
