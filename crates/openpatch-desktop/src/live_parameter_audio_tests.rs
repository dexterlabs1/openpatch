fn live_parameter_audio_patch() -> Patch {
    serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,"nodes":[
            {"id":"signal","kind":"poly_constant","params":{"v1":5}},
            {"id":"vca","kind":"stock_vca","params":{"level":1}},
            {"id":"filter","kind":"rack264_vcf","params":{"frequency":0.5}}
        ],"cables":[{"from":"signal","to":"vca","input":"signal"}],
        "output":"vca","output_port":"out"
    }))
    .unwrap()
}

#[test]
fn live_parameter_callback_publication_saves_actual_values_and_reopens_without_history_mutation() {
    let patch = live_parameter_audio_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let gain = callback
        .graphs
        .active
        .engine
        .resolve_parameter("vca", "level")
        .unwrap();
    let cutoff = callback
        .graphs
        .active
        .engine
        .resolve_parameter("filter", "frequency")
        .unwrap();
    without_heap_activity(|| {
        callback
            .graphs
            .active
            .engine
            .set_parameter_value(gain, 0.25);
        callback
            .graphs
            .active
            .engine
            .set_parameter_value(cutoff, 0.3);
        callback.render(&mut [0.0_f32; 256], 1);
    });
    let before = session.snapshot();
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["vca"]["parameters"]["level"], 0.25);
    assert_eq!(
        published["modules"]["filter"]["parameters"]["frequency"]
            .as_f64()
            .unwrap() as f32,
        0.3
    );
    session
        .execute(
            "save_document",
            serde_json::json!({"filename":"effective.json"}),
        )
        .unwrap();
    assert_eq!(session.snapshot(), before);
    let mut reopened = openpatch::control::Session::new(workspace.path()).unwrap();
    reopened
        .execute(
            "load_document",
            serde_json::json!({"filename":"effective.json"}),
        )
        .unwrap();
    let mut engine = reopened.patch().compile().unwrap();
    let gain = engine.resolve_parameter("vca", "level").unwrap();
    let cutoff = engine.resolve_parameter("filter", "frequency").unwrap();
    assert_eq!(engine.parameter_value(gain), Some(0.25));
    assert_eq!(engine.parameter_value(cutoff), Some(0.3));
    let mut frames = [openpatch_core::PortFrame::default(); 2];
    engine.process_poly_block(&mut frames);
    assert_eq!(frames[1].values[0], 1.25);
}

#[test]
fn live_parameter_publication_and_graph_retirement_pressure_use_no_callback_heap() {
    let mut patch = live_parameter_audio_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    for revision in 2..=5 {
        patch.nodes[1]
            .params
            .insert("level".into(), revision as f32 / 10.0);
        sync_live_parameter_assignments(&callback, &patch, revision, &[("vca", "level")]);
        let graph = sequence_graph(&callback, &patch, revision);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    }
    // Control-side draining releases pressure. All callback-owned boxes survive
    // until retirement succeeds; none may be freed on the rendering thread.
    while retired.pop().is_ok() {}
    for _ in 0..3 {
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        while retired.pop().is_ok() {}
    }
    let snapshot = callback.performance.snapshot();
    assert_eq!(snapshot["modules"]["vca"]["available"], true);
    assert_eq!(snapshot["modules"]["vca"]["revision"], 5);
    assert_eq!(snapshot["modules"]["vca"]["parameters"]["level"], 0.5);
}

fn sync_live_parameter_assignments(
    callback: &Callback,
    patch: &Patch,
    revision: u64,
    keys: &[(&str, &str)],
) {
    let mut assignments =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, f32>>::new();
    for (id, key) in keys {
        let node = patch.nodes.iter().find(|node| node.id == *id).unwrap();
        assignments
            .entry((*id).into())
            .or_default()
            .insert((*key).into(), node.params[*key]);
    }
    callback
        .performance
        .sync_patch_with_parameters(patch, revision, &assignments);
}

fn set_live_parameter(callback: &mut Callback, id: &str, key: &str, value: f32) {
    let engine = &mut callback.graphs.active.engine;
    let handle = engine.resolve_parameter(id, key).unwrap();
    assert!(engine.set_parameter_value(handle, value));
}

fn assert_live_parameter(callback: &Callback, id: &str, key: &str, expected: f32) {
    let engine = &callback.graphs.active.engine;
    let handle = engine.resolve_parameter(id, key).unwrap();
    assert_eq!(engine.parameter_value(handle), Some(expected));
}

fn render_live_parameter_gain(callback: &mut Callback, expected: f32) {
    let mut audio = [0.0_f32; 2048];
    without_heap_activity(|| callback.render(&mut audio, 1));
    assert_eq!(audio[2047], expected);
}

#[test]
fn live_parameter_actual_callback_manual_and_equal_authored_assignments_win() {
    for assigned in [0.6, 1.0] {
        let mut patch = live_parameter_audio_patch();
        patch.nodes[0].params.insert("v1".into(), 0.5);
        let (mut callback, mut updates, _retired) = sequence_callback(&patch);
        set_live_parameter(&mut callback, "vca", "level", 0.25);
        set_live_parameter(&mut callback, "filter", "frequency", 0.3);
        render_live_parameter_gain(&mut callback, 0.125);
        patch.nodes[1].params.insert("level".into(), assigned);
        sync_live_parameter_assignments(&callback, &patch, 2, &[("vca", "level")]);
        // The ordinary attachment/retry path must preserve the assignment epoch.
        callback.performance.sync_patch(&patch, 2);
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        render_live_parameter_gain(&mut callback, 0.5 * assigned);
        assert_live_parameter(&callback, "vca", "level", assigned);
        assert_live_parameter(&callback, "filter", "frequency", 0.3);
        let snapshot = callback.performance.snapshot();
        assert_eq!(snapshot["modules"]["vca"]["parameters"]["level"], assigned);
    }
}

#[test]
fn live_parameter_actual_callback_unrelated_recompile_inherits_untargeted_values() {
    let mut patch = live_parameter_audio_patch();
    patch.nodes[0].params.insert("v1".into(), 0.5);
    let (mut callback, mut updates, _retired) = sequence_callback(&patch);
    set_live_parameter(&mut callback, "vca", "level", 0.25);
    set_live_parameter(&mut callback, "filter", "frequency", 0.3);
    render_live_parameter_gain(&mut callback, 0.125);
    patch.nodes[0].params.insert("v1".into(), 1.0);
    callback.performance.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    render_live_parameter_gain(&mut callback, 0.25);
    assert_live_parameter(&callback, "vca", "level", 0.25);
    assert_live_parameter(&callback, "filter", "frequency", 0.3);
}

#[test]
fn live_parameter_actual_callback_uses_frozen_intent_and_keeps_skipped_assignments() {
    let mut patch = live_parameter_audio_patch();
    patch.nodes[0].params.insert("v1".into(), 0.5);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    set_live_parameter(&mut callback, "vca", "level", 0.25);
    patch.nodes[1].params.insert("level".into(), 0.8);
    sync_live_parameter_assignments(&callback, &patch, 2, &[("vca", "level")]);
    let earlier = sequence_graph(&callback, &patch, 2);
    patch.nodes[1].params.insert("level".into(), 0.6);
    sync_live_parameter_assignments(&callback, &patch, 3, &[("vca", "level")]);
    let later = sequence_graph(&callback, &patch, 3);
    assert!(updates.push(earlier).is_ok());
    render_live_parameter_gain(&mut callback, 0.4);
    assert_live_parameter(&callback, "vca", "level", 0.8);
    // If adoption consulted revision 3's mutable registry epoch, the later
    // assignment would look consumed and this live value would incorrectly win.
    set_live_parameter(&mut callback, "vca", "level", 0.3);
    assert!(updates.push(later).is_ok());
    render_live_parameter_gain(&mut callback, 0.3);
    assert_live_parameter(&callback, "vca", "level", 0.6);
    while retired.pop().is_ok() {}

    patch.nodes[1].params.insert("level".into(), 0.9);
    sync_live_parameter_assignments(&callback, &patch, 4, &[("vca", "level")]);
    let skipped = sequence_graph(&callback, &patch, 4);
    patch.nodes[0].params.insert("v1".into(), 0.25);
    callback.performance.sync_patch(&patch, 5);
    let latest = sequence_graph(&callback, &patch, 5);
    drop(skipped); // Control-side coalescing drops its prepared allocation.
    set_live_parameter(&mut callback, "vca", "level", 0.2);
    assert!(updates.push(latest).is_ok());
    render_live_parameter_gain(&mut callback, 0.225);
    assert_live_parameter(&callback, "vca", "level", 0.9);
}

#[test]
fn live_parameter_actual_callback_cross_rate_preserves_effective_and_explicit_values() {
    for assign in [false, true] {
        let mut patch = live_parameter_audio_patch();
        patch.nodes[0].params.insert("v1".into(), 0.5);
        let (mut callback, mut updates, _retired) = sequence_callback(&patch);
        set_live_parameter(&mut callback, "vca", "level", 0.25);
        set_live_parameter(&mut callback, "filter", "frequency", 0.3);
        if assign {
            sync_live_parameter_assignments(&callback, &patch, 2, &[("vca", "level")]);
        } else {
            callback.performance.sync_patch(&patch, 2);
        }
        let mut graph = prepare_graph(&patch, 96_000, None, 2, true).unwrap();
        graph.modules = callback.performance.plan(&patch, &graph.engine, 2).unwrap();
        graph.revision = 2;
        assert!(updates.push(graph).is_ok());
        render_live_parameter_gain(&mut callback, if assign { 0.5 } else { 0.125 });
        assert_live_parameter(&callback, "vca", "level", if assign { 1.0 } else { 0.25 });
        assert_live_parameter(&callback, "filter", "frequency", 0.3);
    }
}

#[test]
fn live_parameter_actual_callback_retired_or_host_reset_target_keeps_receiving_values() {
    for host_reset in [false, true] {
        let mut patch = live_parameter_audio_patch();
        patch.nodes[0].params.insert("v1".into(), 0.5);
        let (mut callback, mut updates, _retired) = sequence_callback(&patch);
        set_live_parameter(&mut callback, "vca", "level", 0.25);
        set_live_parameter(&mut callback, "filter", "frequency", 0.3);
        if !host_reset {
            // Delete/recreate coalesced into one receiving graph must still
            // retire the former same-ID incarnation and its effective value.
            callback.performance.retire_modules(&["vca".into()]);
        }
        callback.performance.sync_patch(&patch, 2);
        let mut graph = sequence_graph(&callback, &patch, 2);
        if host_reset {
            let mut delta = HostLifecycleDelta::default();
            delta.modules.entry("vca".into()).or_default().reset = true;
            graph.host_lifecycle = PreparedHostLifecycle::prepare(&graph.engine, delta).unwrap();
        }
        assert!(updates.push(graph).is_ok());
        render_live_parameter_gain(&mut callback, 0.5);
        assert_live_parameter(&callback, "vca", "level", 1.0);
        assert_live_parameter(&callback, "filter", "frequency", 0.3);
    }
}

#[test]
fn live_parameter_shared_session_edit_undo_redo_and_unrelated_history_keep_actual_values() {
    let patch = live_parameter_audio_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    set_live_parameter(&mut callback, "vca", "level", 0.25);
    set_live_parameter(&mut callback, "filter", "frequency", 0.3);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    for (tool, args, expected) in [
        (
            "set_parameter",
            serde_json::json!({"id":"vca","parameter":"level","value":1}),
            1.0,
        ),
        ("undo", serde_json::json!({}), 0.25),
        ("redo", serde_json::json!({}), 1.0),
    ] {
        session.execute(tool, args).unwrap();
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        assert_live_parameter(&callback, "vca", "level", expected);
        assert_live_parameter(&callback, "filter", "frequency", 0.3);
        while retired.pop().is_ok() {}
    }
    // Simulate an automated value change without an authored transaction.
    let authored = session.snapshot();
    set_live_parameter(&mut callback, "vca", "level", 0.4);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    assert_eq!(session.snapshot(), authored);
    for (tool, args) in [
        (
            "set_parameter",
            serde_json::json!({"id":"signal","parameter":"v1","value":4}),
        ),
        ("undo", serde_json::json!({})),
    ] {
        session.execute(tool, args).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        assert_live_parameter(&callback, "vca", "level", 0.4);
        assert_live_parameter(&callback, "filter", "frequency", 0.3);
        while retired.pop().is_ok() {}
    }
    session
        .execute(
            "save_document",
            serde_json::json!({"filename":"parameter-history.json"}),
        )
        .unwrap();
    let mut reopened = openpatch::control::Session::new(workspace.path()).unwrap();
    reopened
        .execute(
            "load_document",
            serde_json::json!({"filename":"parameter-history.json"}),
        )
        .unwrap();
    let mut engine = reopened.patch().compile().unwrap();
    let mut frames = [openpatch_core::PortFrame::default(); 2];
    engine.process_poly_block(&mut frames);
    assert_eq!(frames[1].values[0], 2.0);
}

#[test]
fn live_parameter_shared_session_removed_module_undo_restores_effective_gain() {
    let mut patch = live_parameter_audio_patch();
    patch.nodes[0].params.insert("v1".into(), 0.5);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    set_live_parameter(&mut callback, "vca", "level", 0.25);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    for (tool, args) in [
        ("remove_module", serde_json::json!({"id":"vca"})),
        ("undo", serde_json::json!({})),
    ] {
        session.execute(tool, args).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        while retired.pop().is_ok() {}
    }
    assert_live_parameter(&callback, "vca", "level", 0.25);
    render_live_parameter_gain(&mut callback, 0.125);
}

#[test]
fn live_parameter_execution_suspension_and_resume_keep_effective_values() {
    use openpatch::project_document::ExecutionDisposition;
    let mut patch = live_parameter_audio_patch();
    patch.nodes[0].params.insert("v1".into(), 0.5);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    set_live_parameter(&mut callback, "vca", "level", 0.25);
    set_live_parameter(&mut callback, "filter", "frequency", 0.3);
    render_live_parameter_gain(&mut callback, 0.125);
    for revision in [2, 3] {
        if revision == 2 {
            callback
                .performance
                .request_document_execution(revision, ExecutionDisposition::SuspendedUnresolved);
        }
        callback.performance.sync_patch(&patch, revision);
        assert!(updates
            .push(sequence_graph(&callback, &patch, revision))
            .is_ok());
        let mut silence = [1.0_f32; 512];
        without_heap_activity(|| callback.render(&mut silence, 1));
        assert!(silence.iter().all(|v| *v == 0.0));
        assert_live_parameter(&callback, "vca", "level", 0.25);
        assert_live_parameter(&callback, "filter", "frequency", 0.3);
        assert!(!callback.performance.execution_available());
        while retired.pop().is_ok() {}
    }
    callback
        .performance
        .request_document_execution(4, ExecutionDisposition::RunnablePartial);
    callback.performance.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_live_parameter_gain(&mut callback, 0.125);
    assert!(callback.performance.execution_available());
    assert_live_parameter(&callback, "filter", "frequency", 0.3);
}

#[test]
fn rackmcp_automation_actual_callback_publication_history_and_saved_reopen() {
    let mut patch = live_parameter_audio_patch();
    patch.nodes[0].params.insert("v1".into(), 0.5);
    patch.nodes.insert(0, serde_json::from_value(serde_json::json!({
        "id":"automation","kind":"rackmcp_sequencer","params":{"running":1},
        "rackmcp_sequence":{"data":{"patterns":[{"name":"A","length":4}],
            "lanes":[{"moduleId":"42","paramId":0,"points":[{"bar":0,"value":0.25}]}]},
            "targets":[{"source_module_id":"42","source_param_id":0,"module":"vca","parameter":"level"}]}
    })).unwrap());
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let authored = session.snapshot();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    assert_live_parameter(&callback, "vca", "level", 0.25);
    assert_eq!(session.snapshot(), authored);
    // Real automation writes feed the same fenced save observation as manual
    // edits. Capturing neither mutates the authored document nor adds history.
    session
        .execute(
            "save_document",
            serde_json::json!({"filename":"rackmcp-effective.json"}),
        )
        .unwrap();
    assert_eq!(session.snapshot(), authored);
    let mut reopened = openpatch::control::Session::new(workspace.path()).unwrap();
    reopened
        .execute(
            "load_document",
            serde_json::json!({"filename":"rackmcp-effective.json"}),
        )
        .unwrap();
    assert_eq!(
        reopened.patch().nodes[0].rackmcp_sequence,
        session.patch().nodes[0].rackmcp_sequence
    );
    let mut engine = reopened.patch().compile().unwrap();
    let gain = engine.resolve_parameter("vca", "level").unwrap();
    assert_eq!(engine.parameter_value(gain), Some(0.25));
    let mut frames = [openpatch_core::PortFrame::default(); 64];
    engine.process_poly_block(&mut frames);
    assert_eq!(frames[63].values[0], 0.125);
    // A constant source lane must allow manual overrides until its value
    // changes, including across actual prepared-plan history replacements.
    for (tool, args, expected) in [
        (
            "set_parameter",
            serde_json::json!({"id":"vca","parameter":"level","value":0.75}),
            0.75,
        ),
        ("undo", serde_json::json!({}), 0.25),
        ("redo", serde_json::json!({}), 0.75),
        (
            "set_parameter",
            serde_json::json!({"id":"automation","parameter":"bpm","value":128}),
            0.75,
        ),
        ("undo", serde_json::json!({}), 0.75),
    ] {
        session.execute(tool, args).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        assert_live_parameter(&callback, "vca", "level", expected);
        while retired.pop().is_ok() {}
    }
}
