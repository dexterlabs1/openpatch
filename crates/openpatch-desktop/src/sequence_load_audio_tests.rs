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
        &["automation".into()],
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

fn competing_sequence_load_patch() -> Patch {
    let mut patch = sequence_load_patch();
    let mut other = serde_json::to_value(&patch.nodes[0]).unwrap();
    other["id"] = serde_json::json!("automation_b");
    other["rackmcp_sequence"]["data"]["lanes"][0]["points"][0]["value"] = serde_json::json!(0.75);
    patch
        .nodes
        .insert(1, serde_json::from_value(other).unwrap());
    patch
}

fn sync_ordered_sequence_loads(callback: &Callback, patch: &Patch, revision: u64, ids: &[&str]) {
    let ids: Vec<String> = ids.iter().map(|id| (*id).into()).collect();
    callback.performance.sync_patch_with_intents(
        patch,
        revision,
        &std::collections::BTreeMap::new(),
        &ids,
    );
}

#[test]
fn sequence_load_competing_target_owner_follows_event_order_for_equal_fresh_and_changed_models() {
    for reverse_nodes in [false, true] {
        for lifecycle in ["equal", "fresh", "changed", "host_reset"] {
            for (ids, expected) in [
                (["automation", "automation_b"], 0.75),
                (["automation_b", "automation"], 0.25),
            ] {
                let mut patch = competing_sequence_load_patch();
                if reverse_nodes {
                    patch.nodes.swap(0, 1);
                }
                let (mut callback, mut updates, _retired) = sequence_callback(&patch);
                without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
                if lifecycle == "fresh" {
                    callback
                        .performance
                        .retire_modules(&["automation".into(), "automation_b".into()]);
                }
                if lifecycle == "changed" {
                    for node in &mut patch.nodes {
                        if node.kind == "rackmcp_sequencer" {
                            let mut value = serde_json::to_value(&*node).unwrap();
                            value["rackmcp_sequence"]["data"]["patterns"][0]["length"] =
                                serde_json::json!(8);
                            *node = serde_json::from_value(value).unwrap();
                        }
                    }
                }
                sync_ordered_sequence_loads(&callback, &patch, 2, &ids);
                let mut graph = sequence_graph(&callback, &patch, 2);
                if lifecycle == "host_reset" {
                    let mut delta = HostLifecycleDelta::default();
                    for id in ids {
                        delta.modules.entry(id.into()).or_default().reset = true;
                    }
                    graph.host_lifecycle =
                        PreparedHostLifecycle::prepare(&graph.engine, delta).unwrap();
                }
                assert!(updates.push(graph).is_ok());
                render_sequence_lane(&mut callback, expected);
            }
        }
    }
}

#[test]
fn sequence_load_competing_target_skipped_revisions_keep_persistent_order() {
    let patch = competing_sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    sync_ordered_sequence_loads(&callback, &patch, 2, &["automation_b"]);
    sync_ordered_sequence_loads(&callback, &patch, 3, &["automation"]);
    callback.performance.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    sync_ordered_sequence_loads(&callback, &patch, 5, &["automation"]);
    sync_ordered_sequence_loads(&callback, &patch, 6, &["automation_b"]);
    callback.performance.sync_patch(&patch, 7);
    assert!(updates.push(sequence_graph(&callback, &patch, 7)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
}

#[test]
fn sequence_load_competing_target_late_frozen_plan_never_reclaims() {
    let patch = competing_sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    sync_ordered_sequence_loads(&callback, &patch, 2, &["automation"]);
    let older = sequence_graph(&callback, &patch, 2);
    sync_ordered_sequence_loads(&callback, &patch, 3, &["automation_b"]);
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    assert!(updates.push(older).is_ok());
    render_sequence_lane(&mut callback, 0.6);
    while retired.pop().is_ok() {}
    callback.performance.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.6);
}

#[test]
fn sequence_load_competing_owner_survives_graph_reorder_rate_and_winner_deletion() {
    let mut patch = competing_sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    sync_ordered_sequence_loads(&callback, &patch, 2, &["automation_b", "automation"]);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    patch.nodes.swap(0, 1);
    patch.nodes[0].params.insert("bpm".into(), 128.0);
    patch.nodes.push(
        serde_json::from_value(serde_json::json!({
            "id":"unrelated","kind":"constant","params":{"value":0}
        }))
        .unwrap(),
    );
    callback.performance.sync_patch(&patch, 3);
    let mut graph = prepare_graph(&patch, 96_000, None, 3, true).unwrap();
    graph.modules = callback.performance.plan(&patch, &graph.engine, 3).unwrap();
    graph.revision = 3;
    assert!(updates.push(graph).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    patch.nodes.retain(|node| node.id != "automation");
    callback.performance.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    render_sequence_lane(&mut callback, 0.6);
}

#[test]
fn sequence_load_shared_competing_equal_presets_batch_undo_redo_follow_action_order() {
    let patch = competing_sequence_load_patch();
    let preset_a = openpatch::presets::capture(&patch.nodes[0], None).unwrap();
    let preset_b = openpatch::presets::capture(&patch.nodes[1], None).unwrap();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    for (tool, arguments, expected) in [
        (
            "apply_edits",
            serde_json::json!({"edits":[
                {"tool":"apply_module_preset","arguments":{"id":"automation","preset":preset_a}},
                {"tool":"apply_module_preset","arguments":{"id":"automation_b","preset":preset_b}}
            ]}),
            0.75,
        ),
        ("undo", serde_json::json!({}), 0.25),
        ("redo", serde_json::json!({}), 0.75),
    ] {
        set_live_parameter(&mut callback, "vca", "level", 0.6);
        session.execute(tool, arguments).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        render_sequence_lane(&mut callback, expected);
        while retired.pop().is_ok() {}
    }
}

fn change_sequence_lane_value(patch: &mut Patch, id: &str, value: f32) {
    let node = patch.nodes.iter_mut().find(|node| node.id == id).unwrap();
    let mut encoded = serde_json::to_value(&*node).unwrap();
    encoded["rackmcp_sequence"]["data"]["lanes"][0]["points"][0]["value"] =
        serde_json::json!(value);
    *node = serde_json::from_value(encoded).unwrap();
}

#[test]
fn sequence_load_changed_data_frozen_plans_allow_forward_order_and_reject_rollback() {
    for newest_first in [false, true] {
        let patch = competing_sequence_load_patch();
        let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        let mut older_patch = patch.clone();
        change_sequence_lane_value(&mut older_patch, "automation", 0.9);
        sync_ordered_sequence_loads(&callback, &older_patch, 2, &["automation"]);
        let older = sequence_graph(&callback, &older_patch, 2);
        sync_ordered_sequence_loads(&callback, &patch, 3, &["automation_b"]);
        let newer = sequence_graph(&callback, &patch, 3);
        if newest_first {
            assert!(updates.push(newer).is_ok());
            render_sequence_lane(&mut callback, 0.75);
            while retired.pop().is_ok() {}
            assert!(updates.push(older).is_ok());
            render_sequence_lane(&mut callback, 0.75);
            assert_eq!(callback.graphs.active.revision, 3);
            assert!(callback.graphs.previous.is_none());
        } else {
            // Preparation predates the newer registry event, but the frozen
            // graph remains valid while the active document is still older.
            assert!(updates.push(older).is_ok());
            render_sequence_lane(&mut callback, 0.9);
            assert_eq!(callback.graphs.active.revision, 2);
            while retired.pop().is_ok() {}
            assert!(updates.push(newer).is_ok());
            render_sequence_lane(&mut callback, 0.75);
            assert_eq!(callback.graphs.active.revision, 3);
        }
    }
}

#[test]
fn sequence_load_stale_graph_cannot_remove_the_current_winner() {
    let patch = competing_sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    let mut older_patch = patch.clone();
    older_patch.nodes.retain(|node| node.id != "automation");
    callback.performance.sync_patch(&older_patch, 2);
    let older = sequence_graph(&callback, &older_patch, 2);
    sync_ordered_sequence_loads(&callback, &patch, 3, &["automation"]);
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    assert!(updates.push(older).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    assert_eq!(callback.graphs.active.revision, 3);
    assert!(callback
        .graphs
        .active
        .engine
        .resolve_module_state("automation")
        .is_ok());
}

#[test]
fn sequence_load_stale_new_incarnation_cannot_reappear_after_all_sequences_are_removed() {
    let patch = competing_sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    callback.performance.retire_modules(&["automation".into()]);
    sync_ordered_sequence_loads(&callback, &patch, 2, &["automation"]);
    let older = sequence_graph(&callback, &patch, 2);
    let mut removed = patch.clone();
    removed
        .nodes
        .retain(|node| node.kind != "rackmcp_sequencer");
    callback.performance.sync_patch(&removed, 3);
    assert!(updates.push(sequence_graph(&callback, &removed, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    assert!(updates.push(older).is_ok());
    render_sequence_lane(&mut callback, 0.6);
    assert_eq!(callback.graphs.active.revision, 3);
    assert!(callback
        .graphs
        .active
        .engine
        .resolve_module_state("automation")
        .is_err());
    assert!(callback
        .graphs
        .active
        .engine
        .resolve_module_state("automation_b")
        .is_err());
}

#[test]
fn sequence_load_rejected_graph_retirement_backpressure_retains_every_engine_without_heap() {
    let patch = competing_sequence_load_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    sync_ordered_sequence_loads(&callback, &patch, 2, &["automation"]);
    let older_one = sequence_graph(&callback, &patch, 2);
    let older_two = sequence_graph(&callback, &patch, 2);
    sync_ordered_sequence_loads(&callback, &patch, 3, &["automation_b"]);
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    // One retired donor plus the first rejected graph fill the two-slot queue.
    assert!(updates.push(older_one).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    assert!(updates.push(older_two).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    assert_eq!(callback.graphs.active.revision, 3);
    assert_eq!(
        callback
            .graphs
            .waiting_retirement
            .as_ref()
            .unwrap()
            .revision,
        2
    );
    assert!(callback.graphs.previous.is_none());
    sync_ordered_sequence_loads(&callback, &patch, 4, &["automation"]);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    assert_eq!(callback.graphs.active.revision, 3);
    let mut retired_count = 0;
    while retired.pop().is_ok() {
        retired_count += 1;
    }
    assert_eq!(retired_count, 2);
    render_sequence_lane(&mut callback, 0.25);
    assert_eq!(callback.graphs.active.revision, 4);
    assert!(callback.graphs.waiting_retirement.is_none());
    while retired.pop().is_ok() {
        retired_count += 1;
    }
    // Initial donor, both rejected graphs, and revision 3 all return to control.
    assert_eq!(retired_count, 4);
}

#[test]
fn sequence_load_shared_creation_and_duplication_commit_the_last_claim_through_history() {
    let full = competing_sequence_load_patch();
    let mut initial = full.clone();
    initial
        .nodes
        .retain(|node| node.kind != "rackmcp_sequencer");
    let (mut callback, mut updates, mut retired) = sequence_callback(&initial);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(initial, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    let add = |node: &openpatch_core::Node| {
        serde_json::json!({
            "tool":"add_module","arguments":{"id":node.id,"kind":node.kind,
                "params":node.params,"rackmcp_sequence":node.rackmcp_sequence}
        })
    };
    session.execute("apply_edits", serde_json::json!({"edits":[
        add(&full.nodes[0]), add(&full.nodes[1]),
        {"tool":"set_rackmcp_sequence","arguments":{"id":"automation","rackmcp_sequence":full.nodes[0].rackmcp_sequence}}
    ]})).unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    session.execute("undo", serde_json::json!({})).unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    render_sequence_lane(&mut callback, 0.6);
    session.execute("redo", serde_json::json!({})).unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    session.execute("apply_edits", serde_json::json!({"edits":[
        {"tool":"duplicate_modules","arguments":{"modules":[{"id":"automation","new_id":"copy_z"},{"id":"automation_b","new_id":"copy_a"}]}},
        {"tool":"set_rackmcp_sequence","arguments":{"id":"copy_z","rackmcp_sequence":full.nodes[0].rackmcp_sequence}}
    ]})).unwrap();
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    session.execute("undo", serde_json::json!({})).unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    render_sequence_lane(&mut callback, 0.6);
    session.execute("redo", serde_json::json!({})).unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
}

#[test]
fn sequence_load_revision_fence_survives_an_intermediate_nonsequence_rollback() {
    let sequenced = competing_sequence_load_patch();
    let mut unsequenced = sequenced.clone();
    unsequenced
        .nodes
        .retain(|node| node.kind != "rackmcp_sequencer");
    let (mut callback, mut updates, mut retired) = sequence_callback(&unsequenced);
    callback.performance.sync_patch(&unsequenced, 2);
    let older_unsequenced = sequence_graph(&callback, &unsequenced, 2);
    sync_ordered_sequence_loads(&callback, &sequenced, 3, &["automation_b", "automation"]);
    let older_sequenced = sequence_graph(&callback, &sequenced, 3);
    assert!(updates
        .push(sequence_graph(&callback, &sequenced, 3))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    callback.performance.sync_patch(&unsequenced, 4);
    assert!(updates
        .push(sequence_graph(&callback, &unsequenced, 4))
        .is_ok());
    render_sequence_lane(&mut callback, 0.25);
    assert_eq!(callback.graphs.active.revision, 4);
    while retired.pop().is_ok() {}
    // Existing non-sequence graph behavior permits this rollback, but it must
    // not lower the fence that prevents removed sequencing from reappearing.
    assert!(updates.push(older_unsequenced).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    assert_eq!(callback.graphs.active.revision, 2);
    assert_eq!(callback.graph_revision_high_water, 4);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    assert!(updates.push(older_sequenced).is_ok());
    render_sequence_lane(&mut callback, 0.6);
    assert_eq!(callback.graphs.active.revision, 2);
    assert_eq!(callback.graph_revision_high_water, 4);
    assert!(callback
        .graphs
        .active
        .engine
        .resolve_module_state("automation")
        .is_err());
    assert!(callback.graphs.previous.is_none());
}
