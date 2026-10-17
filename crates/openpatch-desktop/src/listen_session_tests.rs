// One imported musical workflow through actual shared MCP tools and the native
// callback/worker; this is a declared Partial excerpt, without audio hardware.
#[test]
fn imported_musical_listen_shared_tools_edit_history_capture_and_source_preserving_reopen() {
    use openpatch::control::{execute_shared, Session, SharedSession};
    use openpatch::project_document::{ExecutionDisposition, PlaybackMode, ProjectDocument};
    use serde_json::json;

    let source = json!({"version":"2.6.6","future":{"keep":[null,"original source"]},
    "modules":[
        {"id":11,"plugin":"RackMCP","model":"Sequencer","version":"2.0.0","pos":[0,0],
            "params":[{"id":2,"value":1}],"data":{"seed":1729,
                "patterns":[{"name":"A","length":4}],
                "lanes":[{"moduleId":"42","paramId":0,"mode":"raw",
                    "points":[{"bar":0,"value":0.25}]}]}},
        {"id":21,"plugin":"Fundamental","model":"VCO","version":"2.6.4","pos":[10,0],
            "params":[{"id":2,"value":0}]},
        {"id":42,"plugin":"Fundamental","model":"VCA-1","version":"2.6.4","pos":[20,0],
            "params":[{"id":0,"value":1},{"id":1,"value":1}]},
        {"id":77,"plugin":"RackMCP","model":"Listen","version":"2.0.0","pos":[30,0],
            "future":{"retain":"Listen payload"}},
        {"id":99,"plugin":"UnsupportedFixture","model":"PreservedOnly","version":"1.0.0","pos":[40,0],
            "data":{"opaque":[1,2,3]}}
    ],"cables":[
        {"id":1,"outputModuleId":21,"outputId":0,"inputModuleId":42,"inputId":1},
        {"id":2,"outputModuleId":42,"outputId":0,"inputModuleId":77,"inputId":0}
    ]});
    let original = serde_json::to_vec_pretty(&source).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("musical-listen.vcv"), &original).unwrap();
    let shared = Session::new(workspace.path()).unwrap().into_shared();
    let imported = execute_shared(
        &shared,
        "import_vcv",
        json!({"filename":"musical-listen.vcv"}),
    )
    .unwrap();
    assert_eq!(imported["report"]["mapped_count"], 4);
    assert_eq!(imported["report"]["unresolved_count"], 1);
    let (sources, blobs) = {
        let session = shared.lock().unwrap();
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        assert_eq!(
            session
                .patch()
                .nodes
                .iter()
                .find(|node| node.id == "vcv_m_77")
                .unwrap()
                .kind,
            "rackmcp_listen"
        );
        (
            serde_json::to_value(&session.project_document().sources).unwrap(),
            session.project_document().source_blobs.clone(),
        )
    };
    let assert_source = |document: &ProjectDocument| {
        document.validate_structure().unwrap();
        assert_eq!(serde_json::to_value(&document.sources).unwrap(), sources);
        assert_eq!(document.source_blobs.len(), blobs.len());
        for (id, blob) in &blobs {
            assert_eq!(document.source_blobs[id].bytes(), blob.bytes());
        }
        let package = document.sources.values().next().unwrap();
        assert_eq!(package.patch.as_ref(), &source);
        assert_eq!(
            document.source_blobs[&package.assets["patch.json"]].bytes(),
            original
        );
    };
    execute_shared(
        &shared,
        "set_output",
        json!({"id":"vcv_m_77","port":"left"}),
    )
    .unwrap();
    assert!(execute_shared(&shared, "start_listen_capture", json!({"seconds":0.1})).is_err());
    execute_shared(&shared, "set_document_playback", json!({"mode":"partial"})).unwrap();
    let (patch, revision, disposition) = {
        let session = shared.lock().unwrap();
        (
            session.patch().clone(),
            session.revision(),
            session.execution_disposition(),
        )
    };
    assert_eq!(disposition, ExecutionDisposition::RunnablePartial);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    callback
        .prepare_initial_execution(&patch, revision, disposition)
        .unwrap();
    shared
        .lock()
        .unwrap()
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let graph_for = |callback: &Callback, shared: &SharedSession| {
        let session = shared.lock().unwrap();
        sequence_graph(callback, session.patch(), session.revision())
    };
    let poll_until_done = |callback: &mut Callback, shared: &SharedSession, job: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
        loop {
            without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
            let result =
                execute_shared(shared, "inspect_listen_capture", json!({"job":job})).unwrap();
            if matches!(result["capture"]["state"].as_str(), Some("done" | "failed")) {
                assert_eq!(result["capture"]["state"], "done", "{result}");
                return result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "capture did not complete: {result}"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    let mut audio = [0.0_f32; 2048];
    without_heap_activity(|| callback.render(&mut audio, 1));
    assert!(audio.iter().all(|value| value.is_finite()));
    assert!(audio.iter().any(|value| value.abs() > 0.001));
    assert_live_parameter(&callback, "vcv_m_42", "level", 0.25);
    assert_eq!(
        execute_shared(&shared, "inspect_listen", json!({"id":"vcv_m_77"})).unwrap()["available"],
        true
    );
    let before_capture = shared.lock().unwrap().snapshot();
    let started = execute_shared(
        &shared,
        "start_listen_capture",
        json!({"seconds":0.2,"write_wav":true}),
    )
    .unwrap();
    assert_eq!(shared.lock().unwrap().snapshot(), before_capture);
    let job = started["capture"]["job"].as_str().unwrap();
    assert_eq!(started["capture"]["target_frames"], 9600);

    let armed_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
        if callback.performance.snapshot()["modules"]["vcv_m_77"]["listen"]["capturing"] == true {
            break;
        }
        assert!(
            std::time::Instant::now() < armed_deadline,
            "imported Listen never armed"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    assert_eq!(execute_shared(&shared, "inspect_listen", json!({"id":"vcv_m_77"})).unwrap()["capture"]["job"], job);
    // Ordinary imported oscillator edits and history affect the real voice while
    // the independent host capture remains the same job and fixed target.
    for (tool, args, expected) in [
        (
            "set_parameter",
            json!({"id":"vcv_m_21","parameter":"frequency","value":12}),
            12.0,
        ),
        ("undo", json!({}), 0.0),
        ("redo", json!({}), 12.0),
    ] {
        execute_shared(&shared, tool, args).unwrap();
        assert!(updates.push(graph_for(&callback, &shared)).is_ok());
        without_heap_activity(|| callback.render(&mut audio, 1));
        assert_eq!(
            shared
                .lock()
                .unwrap()
                .patch()
                .nodes
                .iter()
                .find(|node| node.id == "vcv_m_21")
                .unwrap()
                .params["frequency"],
            expected
        );
        // This source VCO has no live parameter getter. Verify the audible
        // octave change itself after the short host crossfade has settled.
        let crossings = audio[256..]
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!(
            (if expected == 0.0 { 8..=12 } else { 17..=22 }).contains(&crossings),
            "{tool}: {crossings} crossings"
        );
        assert!(audio.iter().any(|value| value.abs() > 0.001));
        while retired.pop().is_ok() {}
    }
    let after_history = shared.lock().unwrap().snapshot();
    let captured = poll_until_done(&mut callback, &shared, job);
    assert_eq!(shared.lock().unwrap().snapshot(), after_history);
    let capture = &captured["capture"];
    assert_eq!(capture["sample_rate"], 48000);
    assert_eq!(capture["collected_frames"], 9600);
    assert_eq!(capture["result"]["analysis"]["frames"], 9600);
    assert!(
        capture["result"]["analysis"]["channels"][0]["rmsVolts"]
            .as_f64()
            .unwrap()
            > 0.01
    );
    assert_eq!(capture["result"]["left_connected"], true);
    assert_eq!(capture["result"]["right_connected"], false);
    let wav = capture["result"]["wav_file"].as_str().unwrap();
    assert_eq!(
        std::fs::metadata(workspace.path().join(wav)).unwrap().len(),
        58 + 9600 * 8
    );
    execute_shared(
        &shared,
        "save_document",
        json!({"filename":"musical-listen.json"}),
    )
    .unwrap();
    assert_eq!(shared.lock().unwrap().snapshot(), after_history);
    let saved = ProjectDocument::decode(
        &std::fs::read(workspace.path().join("musical-listen.json")).unwrap(),
    )
    .unwrap();
    assert_source(&saved);
    assert_source(shared.lock().unwrap().project_document());
    callback.performance.close();

    let reopened = Session::new(workspace.path()).unwrap().into_shared();
    execute_shared(
        &reopened,
        "load_document",
        json!({"filename":"musical-listen.json"}),
    )
    .unwrap();
    {
        let session = reopened.lock().unwrap();
        assert_eq!(session.playback_mode(), PlaybackMode::Exact);
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        assert_source(session.project_document());
        let listen = session
            .patch()
            .nodes
            .iter()
            .find(|node| node.id == "vcv_m_77")
            .unwrap();
        assert_eq!(listen.kind, "rackmcp_listen");
        assert!(listen.params.is_empty());
        assert_eq!(
            session
                .patch()
                .nodes
                .iter()
                .find(|node| node.id == "vcv_m_21")
                .unwrap()
                .params["frequency"],
            12.0
        );
    }
    assert!(execute_shared(&reopened, "start_listen_capture", json!({"seconds":0.1})).is_err());
    execute_shared(
        &reopened,
        "set_document_playback",
        json!({"mode":"partial"}),
    )
    .unwrap();
    let (patch, revision, disposition) = {
        let session = reopened.lock().unwrap();
        (
            session.patch().clone(),
            session.revision(),
            session.execution_disposition(),
        )
    };
    let (mut fresh, _, _fresh_retired) = sequence_callback(&patch);
    fresh
        .prepare_initial_execution(&patch, revision, disposition)
        .unwrap();
    reopened
        .lock()
        .unwrap()
        .set_live_module_events(Some(fresh.performance.clone()))
        .unwrap();
    without_heap_activity(|| fresh.render(&mut audio, 1));
    assert_eq!(
        execute_shared(&reopened, "inspect_listen", json!({"id":"vcv_m_77"})).unwrap()["available"],
        true
    );
    let started =
        execute_shared(&reopened, "start_listen_capture", json!({"seconds":0.1})).unwrap();
    let restored = poll_until_done(
        &mut fresh,
        &reopened,
        started["capture"]["job"].as_str().unwrap(),
    );
    assert!(
        restored["capture"]["result"]["analysis"]["channels"][0]["rmsVolts"]
            .as_f64()
            .unwrap()
            > 0.01
    );
    assert_source(reopened.lock().unwrap().project_document());
    fresh.performance.close();
}
