#[test]
fn imported_rackmcp_transport_shared_session_history_and_source_preserving_save_reopen() {
    use openpatch::control::{execute_shared, Session, SharedSession};
    use openpatch::project_document::{DocumentNode, ExecutionDisposition, ProjectDocument};
    use serde_json::json;

    let source = json!({"version":"2.6.6","future":{"retain":[null,17]},"modules":[
        {"id":11,"plugin":"RackMCP","model":"Sequencer","version":"2.0.0",
            "params":[{"id":2,"value":0}],
            "data":{"seed":1729,"patterns":[{"name":"A","length":4},{"name":"B♭","length":4}],
                "lanes":[{"moduleId":"42","paramId":0,"mode":"raw",
                    "points":[{"bar":0,"value":0.25}]}]}},
        {"id":21,"plugin":"Fundamental","model":"VCO","version":"2.6.4",
            "params":[{"id":2,"value":0}]},
        {"id":42,"plugin":"Fundamental","model":"VCA-1","version":"2.6.4",
            "params":[{"id":0,"value":1},{"id":1,"value":1}]}
        ],"cables":[{"id":1,"outputModuleId":21,"outputId":0,
            "inputModuleId":42,"inputId":1}]});
    let original_bytes = serde_json::to_vec_pretty(&source).unwrap();
    let imported = openpatch::vcv_import::import(&original_bytes).unwrap();
    assert_eq!(imported.report.mapped_count, 3);
    assert_eq!(imported.report.unresolved_count, 0);
    let mut document = imported.document;
    let original_sources = serde_json::to_value(&document.sources).unwrap();
    let original_blobs = document.source_blobs.clone();
    let rename = |id: &str| match id {
        "vcv_m_11" => "automation",
        "vcv_m_21" => "oscillator",
        "vcv_m_42" => "vca",
        _ => panic!("unexpected imported module"),
    };
    for entry in &mut document.graph.nodes {
        if let DocumentNode::Native { node, .. } = entry {
            node.id = rename(&node.id).into();
            if let Some(sequence) = &mut node.rackmcp_sequence {
                sequence.targets[0].module = "vca".into();
            }
        }
    }
    for cable in &mut document.graph.cables {
        cable.route.from = rename(&cable.route.from).into();
        cable.route.to = rename(&cable.route.to).into();
    }
    document.graph.output = Some("vca".into());
    document.graph.output_port = "out".into();
    document.validate_structure().unwrap();
    let assert_source = |document: &ProjectDocument| {
        document.validate_structure().unwrap();
        assert_eq!(
            serde_json::to_value(&document.sources).unwrap(),
            original_sources
        );
        assert_eq!(document.source_blobs.len(), original_blobs.len());
        for (id, blob) in &original_blobs {
            assert_eq!(document.source_blobs[id].bytes(), blob.bytes());
        }
        let package = document.sources.values().next().unwrap();
        assert_eq!(package.patch.as_ref(), &source);
        assert_eq!(
            document.source_blobs[&package.assets["patch.json"]].bytes(),
            original_bytes
        );
    };
    let workspace = tempfile::tempdir().unwrap();
    let mut session = Session::new(workspace.path()).unwrap();
    session.replace_project_document(document, Some(0)).unwrap();
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::RunnableExact
    );
    let (mut callback, mut updates, mut retired) = sequence_callback(session.patch());
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let shared = session.into_shared();
    let graph_for = |callback: &Callback, shared: &SharedSession| {
        let session = shared.lock().unwrap();
        sequence_graph(callback, session.patch(), session.revision())
    };
    let sequence_for = |shared: &SharedSession| {
        shared
            .lock()
            .unwrap()
            .patch()
            .nodes
            .iter()
            .find(|node| node.id == "automation")
            .unwrap()
            .rackmcp_sequence
            .clone()
            .unwrap()
    };
    let inspect = |shared: &SharedSession| {
        execute_shared(
            shared,
            "inspect_rackmcp_sequence",
            json!({"id":"automation"}),
        )
        .unwrap()
    };
    let selected_in = |document: &ProjectDocument| {
        let carrier = document.state_carrier().unwrap();
        let node = carrier
            .patch
            .nodes
            .iter()
            .find(|node| node.id == "automation")
            .unwrap();
        assert_eq!(node.params["running"], 1.0);
        node.rackmcp_sequence
            .as_ref()
            .unwrap()
            .data
            .selected_pattern
            .clone()
            .unwrap()
    };
    assert_eq!(inspect(&shared)["available"], false);
    let mut audio = [0.0_f32; 2048];
    without_heap_activity(|| callback.render(&mut audio, 1));
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.001));
    assert_eq!(inspect(&shared)["rackmcp_sequence"]["running"], false);
    let authored_before_transport = shared.lock().unwrap().snapshot();
    let receipt = execute_shared(
        &shared,
        "rackmcp_transport",
        json!({
            "id":"automation","action":"play","pattern":"B♭","quantize":"now"
        }),
    )
    .unwrap();
    assert_eq!(receipt["receipt"]["queued"], true);
    assert_eq!(receipt["receipt"]["applied"], false);
    assert_eq!(shared.lock().unwrap().snapshot(), authored_before_transport);
    without_heap_activity(|| callback.render(&mut audio, 1));
    let pending = inspect(&shared);
    assert_eq!(pending["available"], true);
    assert_eq!(pending["rackmcp_sequence"]["selected_pattern_name"], "B♭");
    assert_eq!(pending["rackmcp_sequence"]["pending_pattern_name"], "B♭");
    assert_eq!(pending["rackmcp_sequence"]["active_pattern_name"], "A");
    assert_eq!(
        pending["rackmcp_sequence"],
        callback.performance.snapshot()["modules"]["automation"]["rackmcp_sequence"]
    );
    // `now` means the next source advance, reached after 6000 running samples.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 4096], 1));
    assert_eq!(
        inspect(&shared)["rackmcp_sequence"]["active_pattern_name"],
        "B♭"
    );
    assert_eq!(shared.lock().unwrap().snapshot(), authored_before_transport);
    {
        let session = shared.lock().unwrap();
        let captured = session
            .capture_project_document(Some(session.revision()))
            .unwrap();
        assert_eq!(selected_in(&captured.document), "B♭");
        assert_source(&captured.document);
    }

    execute_shared(
        &shared,
        "set_parameter",
        json!({"id":"vca","parameter":"level","value":0.6}),
    )
    .unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    render_sequence_lane(&mut callback, 0.6);
    while retired.pop().is_ok() {}
    execute_shared(
        &shared,
        "rackmcp_transport",
        json!({"id":"automation","action":"select","pattern":"A","quantize":"now"}),
    )
    .unwrap();
    without_heap_activity(|| callback.render(&mut audio, 1));
    execute_shared(&shared, "undo", json!({})).unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    without_heap_activity(|| callback.render(&mut audio, 1));
    assert_eq!(
        inspect(&shared)["rackmcp_sequence"]["selected_pattern_name"],
        "A",
        "ordinary parameter undo must preserve the subsequent live selection"
    );
    while retired.pop().is_ok() {}

    let mut changed = sequence_for(&shared);
    changed.data.selected_pattern = Some("B♭".into());
    changed.data.patterns[0].length = 8;
    changed.data.lanes[0].points[0].value = 0.35;
    execute_shared(
        &shared,
        "set_rackmcp_sequence",
        json!({"id":"automation","rackmcp_sequence":changed}),
    )
    .unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    render_sequence_lane(&mut callback, 0.35);
    assert_eq!(
        inspect(&shared)["rackmcp_sequence"]["selected_pattern_name"],
        "B♭"
    );
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    execute_shared(&shared, "undo", json!({})).unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    assert_eq!(
        inspect(&shared)["rackmcp_sequence"]["selected_pattern_name"],
        "A",
        "sequence-data undo restores the live name captured before replacement"
    );
    assert_eq!(
        sequence_for(&shared).data.selected_pattern.as_deref(),
        Some("A")
    );
    while retired.pop().is_ok() {}
    // The history data load rearms the old lane once; ordinary adoption retains
    // a subsequent manual override instead of replaying that load epoch.
    set_live_parameter(&mut callback, "vca", "level", 0.65);
    let revision = shared.lock().unwrap().revision();
    callback
        .performance
        .sync_patch(shared.lock().unwrap().patch(), revision);
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    render_sequence_lane(&mut callback, 0.65);
    while retired.pop().is_ok() {}

    execute_shared(
        &shared,
        "rackmcp_transport",
        json!({"id":"automation","action":"play","pattern":"B♭","quantize":"now"}),
    )
    .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 6144], 1));
    assert_eq!(
        inspect(&shared)["rackmcp_sequence"]["active_pattern_name"],
        "B♭"
    );
    let authored_before_save = shared.lock().unwrap().snapshot();
    execute_shared(
        &shared,
        "save_document",
        json!({"filename":"rackmcp-transport.json"}),
    )
    .unwrap();
    assert_eq!(shared.lock().unwrap().snapshot(), authored_before_save);
    let bytes = std::fs::read(workspace.path().join("rackmcp-transport.json")).unwrap();
    let saved = ProjectDocument::decode(&bytes).unwrap();
    assert_eq!(selected_in(&saved), "B♭");
    assert_source(&saved);
    assert_eq!(saved.encode().unwrap(), bytes);

    let mut reopened = Session::new(workspace.path()).unwrap();
    reopened
        .execute(
            "load_document",
            json!({"filename":"rackmcp-transport.json"}),
        )
        .unwrap();
    assert_eq!(
        reopened.execution_disposition(),
        ExecutionDisposition::RunnableExact
    );
    assert_source(reopened.project_document());
    assert_eq!(selected_in(reopened.project_document()), "B♭");
    let (mut fresh_callback, _, _fresh_retired) = sequence_callback(reopened.patch());
    reopened
        .set_live_module_events(Some(fresh_callback.performance.clone()))
        .unwrap();
    let fresh_shared = reopened.into_shared();
    assert_eq!(inspect(&fresh_shared)["available"], false);
    without_heap_activity(|| fresh_callback.render(&mut audio, 1));
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.001));
    let fresh = inspect(&fresh_shared);
    assert_eq!(fresh["available"], true);
    assert_eq!(fresh["rackmcp_sequence"]["running"], true);
    assert_eq!(fresh["rackmcp_sequence"]["selected_pattern_name"], "B♭");
    assert_live_parameter(&fresh_callback, "vca", "level", 0.25);
}
