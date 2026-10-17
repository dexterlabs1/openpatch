#[test]
fn imported_sequence_shared_session_callback_preserves_bindings_and_gates_saved_partial_audio() {
    use openpatch::control::{execute_shared, Session, SharedSession};
    use openpatch::project_document::{
        DocumentNode, ExecutionDisposition, PlaybackMode, ProjectDocument,
    };
    use serde_json::json;

    let source = json!({"version":"2.6.6","future":{"retain":[null,17]},"modules":[
        {"id":11,"plugin":"RackMCP","model":"Sequencer","version":"2.0.0",
            "params":[{"id":2,"value":1}],
            "data":{"seed":1729,"patterns":[{"name":"A","length":4}],
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
    // Native IDs are editable; source numeric IDs and their package remain the
    // authority for the imported binding after a native rename.
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
    let workspace = tempfile::tempdir().unwrap();
    let mut session = Session::new(workspace.path()).unwrap();
    session.replace_project_document(document, Some(0)).unwrap();
    assert_eq!(session.revision(), 1);
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
    let assert_source = |document: &ProjectDocument| {
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

    let mut audio = [0.0_f32; 2048];
    without_heap_activity(|| callback.render(&mut audio, 1));
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.001));
    assert_live_parameter(&callback, "vca", "level", 0.25);
    while retired.pop().is_ok() {}

    execute_shared(
        &shared,
        "set_parameter",
        json!({"id":"vca","parameter":"level","value":0.6}),
    )
    .unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    render_sequence_lane(&mut callback, 0.6);
    while retired.pop().is_ok() {}
    // A constant lane acknowledges its write once, so subsequent samples keep
    // the real manual edit until an explicit sequence-data load rearms it.
    render_sequence_lane(&mut callback, 0.6);
    let equal_sequence = sequence_for(&shared);
    execute_shared(
        &shared,
        "set_rackmcp_sequence",
        json!({"id":"automation","rackmcp_sequence":equal_sequence}),
    )
    .unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}

    let mut edited_sequence = sequence_for(&shared);
    edited_sequence.data.lanes.push(
        serde_json::from_value(json!({
            "moduleId":"42","paramId":1,"points":[{"bar":0,"value":0.5}]
        }))
        .unwrap(),
    );
    // Capture persists the effective default selection as an explicit typed
    // name. Normalize that field while retaining full lane/target equality.
    let mut expected_node = shared
        .lock()
        .unwrap()
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == "automation")
        .unwrap()
        .clone();
    expected_node.rackmcp_sequence = Some(edited_sequence.clone());
    openpatch_core::SavedModuleState::from_node(&expected_node)
        .unwrap()
        .write_node(&mut expected_node);
    let expected_sequence = expected_node.rackmcp_sequence.unwrap();
    assert_eq!(
        expected_sequence.data.selected_pattern.as_deref(),
        Some("A")
    );
    execute_shared(
        &shared,
        "set_rackmcp_sequence",
        json!({"id":"automation","rackmcp_sequence":edited_sequence}),
    )
    .unwrap();
    {
        let session = shared.lock().unwrap();
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        let status = session.document_execution_status();
        assert_eq!(status["unresolved_modules"], json!([]));
        assert_eq!(status["omitted_automation"].as_array().unwrap().len(), 1);
        assert_eq!(status["omitted_automation"][0]["reason"], "unbound_lane");
    }
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    let mut silence = [1.0_f32; 2048];
    without_heap_activity(|| callback.render(&mut silence, 1));
    assert!(silence.iter().all(|value| *value == 0.0));
    assert!(!callback.performance.execution_available());
    while retired.pop().is_ok() {}

    execute_shared(&shared, "set_document_playback", json!({"mode":"partial"})).unwrap();
    assert!(updates.push(graph_for(&callback, &shared)).is_ok());
    without_heap_activity(|| callback.render(&mut audio, 1));
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.001));
    assert_live_parameter(&callback, "vca", "level", 0.25);
    assert!(callback.performance.execution_available());
    while retired.pop().is_ok() {}
    let authored = shared.lock().unwrap().snapshot();
    {
        let session = shared.lock().unwrap();
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::RunnablePartial
        );
        let captured = session
            .capture_project_document(Some(session.revision()))
            .unwrap();
        let native = captured.document.state_carrier().unwrap();
        assert_eq!(
            native
                .patch
                .nodes
                .iter()
                .find(|node| node.id == "vca")
                .unwrap()
                .params["level"],
            0.25
        );
        let sequence = native
            .patch
            .nodes
            .iter()
            .find(|node| node.id == "automation")
            .unwrap()
            .rackmcp_sequence
            .as_ref()
            .unwrap();
        assert_eq!(sequence, &expected_sequence);
        assert_eq!((sequence.data.lanes.len(), sequence.targets.len()), (2, 1));
        assert_eq!(native.omitted_automation.len(), 1);
        assert_source(&captured.document);
    }
    execute_shared(
        &shared,
        "save_document",
        json!({"filename":"imported-partial.json"}),
    )
    .unwrap();
    assert_eq!(shared.lock().unwrap().snapshot(), authored);
    let bytes = std::fs::read(workspace.path().join("imported-partial.json")).unwrap();
    let saved = ProjectDocument::decode(&bytes).unwrap();
    assert_source(&saved);
    assert_eq!(saved.encode().unwrap(), bytes);

    let mut reopened = Session::new(workspace.path()).unwrap();
    reopened
        .execute("load_document", json!({"filename":"imported-partial.json"}))
        .unwrap();
    assert_eq!(reopened.playback_mode(), PlaybackMode::Exact);
    assert_eq!(
        reopened.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_source(reopened.project_document());
    let (mut fresh_callback, mut fresh_updates, mut fresh_retired) =
        sequence_callback(reopened.patch());
    reopened
        .set_live_module_events(Some(fresh_callback.performance.clone()))
        .unwrap();
    let fresh_shared = reopened.into_shared();
    silence.fill(1.0);
    without_heap_activity(|| fresh_callback.render(&mut silence, 1));
    assert!(silence.iter().all(|value| *value == 0.0));
    assert!(!fresh_callback.performance.execution_available());
    execute_shared(
        &fresh_shared,
        "set_document_playback",
        json!({"mode":"partial"}),
    )
    .unwrap();
    assert!(fresh_updates
        .push(graph_for(&fresh_callback, &fresh_shared))
        .is_ok());
    without_heap_activity(|| fresh_callback.render(&mut audio, 1));
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.001));
    assert_live_parameter(&fresh_callback, "vca", "level", 0.25);
    assert_eq!(sequence_for(&fresh_shared), expected_sequence);
    assert_eq!(
        fresh_shared.lock().unwrap().document_execution_status()["omitted_automation"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    while fresh_retired.pop().is_ok() {}
}
