use openpatch::control::Session;
use openpatch::project_document::hosted::{HostedIdentity, HostedModule};
use openpatch::project_document::{DocumentNode, ProjectDocument, ProjectLayout};
use openpatch_core::SavedModuleState;
use serde_json::json;

fn document() -> ProjectDocument {
    let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"constant","params":{"value":0.5}},
                 {"id":"amp","kind":"stock_vca","params":{"level":0.125,"response":1}},
                 {"id":"hold","kind":"push","params":{"holding":0}}],
        "cables":[],"output":"amp"}))
    .unwrap();
    let mut layout = ProjectLayout::default();
    for (index, id) in ["source", "amp", "hold", "binary"].iter().enumerate() {
        layout
            .positions
            .insert((*id).into(), [index as f32 * 150., 0.]);
    }
    let mut doc = ProjectDocument::from_patch(patch, layout).unwrap();
    doc.graph.nodes.push(DocumentNode::Hosted {
        id: "binary".into(),
        source: None,
        module: HostedModule::offset(
            HostedIdentity {
                plugin: "Bogaudio".into(),
                model: "Bogaudio-Offset".into(),
                version: "2.6.47".into(),
                rack_version: "2.6.6".into(),
                plugin_sha256: "264e55e1389274c3873439372c5c51f48f63168bd6c98fb2c6ffec170224b6f6"
                    .into(),
                runtime_sha256: "55dd40f7dff09a6ed7b93374b150f33931034d3754c128da7b209ea37b6aaf1c"
                    .into(),
            },
            0.,
            0.25,
            true,
            false,
        )
        .unwrap(),
    });
    for route in [
        json!({"id":"input","from":"source","output":"out","to":"binary","input":"in"}),
        json!({"id":"output","from":"binary","output":"out","to":"amp","input":"signal"}),
    ] {
        doc.graph
            .cables
            .push(serde_json::from_value(json!({"route":route})).unwrap());
    }
    doc
}
fn real_configuration(destination: &std::path::Path) -> hosted_audio::HostedAudioConfig {
    let path = |key| std::path::PathBuf::from(std::env::var_os(key).expect(key));
    hosted_audio::HostedAudioConfig {
        companion_executable: path("OPENPATCH_HOSTED_COMPANION"),
        sdk_dir: path("OPENPATCH_HOSTED_SDK"),
        plugin_dir: path("OPENPATCH_HOSTED_PLUGIN"),
        workspace_parent: destination.into(),
    }
}
fn render(callback: &mut Callback, frames: usize) {
    std::thread::sleep(Duration::from_millis(5));
    let mut output = vec![0f32; frames];
    super::super::tests::without_heap_activity(|| callback.render(&mut output, 1));
}
fn submit(audio: &mut Audio, session: &mut Session) {
    let ticket = session.audio_snapshot().unwrap();
    audio.submit_document(ticket.clone()).unwrap();
    assert!(session.acknowledge_audio_snapshot(ticket.id));
}
fn native_level(callback: &Callback) -> f32 {
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_parameter("amp", "level")
        .unwrap();
    callback
        .graphs
        .active
        .engine
        .parameter_value(handle)
        .unwrap()
}

#[test]
fn frozen_native_capture_uses_actual_controls_and_rejects_missing_state() {
    let doc = document();
    let native = doc.state_carrier().unwrap().patch;
    let mut engine = native.compile().unwrap();
    let handle = engine.resolve_parameter("amp", "level").unwrap();
    assert!(engine.set_parameter_value(handle, 0.75));
    let hold = engine.resolve_module_state("hold").unwrap();
    assert!(engine.set_module_saved_state(hold, SavedModuleState::PushHold(true)));
    let captured = capture_native_patch(&engine, &native).unwrap();
    assert_eq!(captured.nodes[1].params["level"], 0.75);
    assert_eq!(captured.nodes[2].params["holding"], 1.);
    assert_eq!(native.nodes[1].params["level"], 0.125);
    assert_eq!(native.nodes[2].params["holding"], 0.);
}

#[test]
#[ignore = "requires explicitly supplied real ARM companion, SDK and installed plugin; no device opened"]
fn actual_application_coherent_capture_partial_quantum_backlog_save_reopen_and_resume() {
    let destination = std::path::PathBuf::from(
        std::env::var_os("OPENPATCH_HOSTED_CAPTURE_REVIEW").expect("review path"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    let children = tempfile::tempdir_in(&destination).unwrap();
    let work = tempfile::tempdir_in(&destination).unwrap();
    let mut session = Session::new(work.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    session.request_delayed_hosted_execution(None).unwrap();
    let native = session.project_document().state_carrier().unwrap().patch;
    let (mut audio, mut callback) = super::super::tests::host_test_audio(&native);
    audio
        .hosted_controller
        .set_config(real_configuration(children.path()));
    session
        .set_live_module_events(Some(audio.performance.clone()))
        .unwrap();
    submit(&mut audio, &mut session);
    audio.set_playing(true);
    for _ in 0..8 {
        render(&mut callback, 128);
        audio.service();
    }
    let old_incarnation = audio.hosted_control.as_ref().unwrap().fence().incarnation;
    let level = callback
        .graphs
        .active
        .engine
        .resolve_parameter("amp", "level")
        .unwrap();
    assert!(callback
        .graphs
        .active
        .engine
        .set_parameter_value(level, 0.75));
    let hold = callback
        .graphs
        .active
        .engine
        .resolve_module_state("hold")
        .unwrap();
    assert!(callback
        .graphs
        .active
        .engine
        .set_module_saved_state(hold, SavedModuleState::PushHold(true)));
    render(&mut callback, 37);
    audio.service();
    for index in 0..130 {
        session
            .execute(
                "set_hosted_parameter",
                json!({"id":"binary","parameter":"offset",
            "value":if index%2==0 {0.25} else {0.5}}),
            )
            .unwrap();
    }
    submit(&mut audio, &mut session);
    let before = json!(session.project_document());
    let history = session.history_state_ids().collect::<Vec<_>>();
    let baseline = session.project_save_baseline().status_from_content(None);
    let ticket = session.begin_hosted_project_capture(None).unwrap();
    let request = audio.begin_hosted_capture(ticket.clone()).unwrap();
    assert!(!audio.performance.execution_available());
    let revision = session.revision();
    assert!(session
        .execute(
            "set_hosted_parameter",
            json!({"id":"binary","parameter":"scale","value":-0.25})
        )
        .is_err());
    assert_eq!(
        session.revision(),
        revision,
        "sealed admission cannot mutate history"
    );
    // A full return queue retains the actual frozen graph on the callback;
    // neither a silent callback nor draining transport may advance it.
    let seal = audio.capture.pending.as_ref().unwrap().seal;
    let silence: Patch = serde_json::from_value(
        json!({"nodes":[{"id":"zero","kind":"constant","params":{"value":0.}}]}),
    )
    .unwrap();
    let dummy = Box::new(CapturePacket {
        seal,
        graph: silence.compile().unwrap().into(),
        state: PacketState::Refused,
    });
    assert!(callback.capture.returns.push(dummy).is_ok());
    for _ in 0..6 {
        render(&mut callback, 128);
        if callback.capture.held_return.is_some() {
            break;
        }
    }
    let held = callback
        .capture
        .held_return
        .as_ref()
        .expect("frozen graph retained under backpressure");
    assert!(matches!(held.state, PacketState::Frozen { .. }));
    let frozen_input = held.graph.hosted.as_ref().unwrap().frame();
    let frozen_native = json!(capture_native_patch(&held.graph.engine, &native).unwrap());
    let device_cut = match held.state {
        PacketState::Frozen { device_frame } => device_frame,
        _ => unreachable!(),
    };
    assert_eq!(
        frozen_input, 1408,
        "finish real input Q8/Q9/Q10; never pad the initial37-frame tail"
    );
    render(&mut callback, 128);
    let held = callback.capture.held_return.as_ref().unwrap();
    assert_eq!(held.graph.hosted.as_ref().unwrap().frame(), frozen_input);
    assert_eq!(
        json!(capture_native_patch(&held.graph.engine, &native).unwrap()),
        frozen_native
    );
    drop(audio.capture.returns.pop().unwrap()); // release only the synthetic blocker, off audio
    render(&mut callback, 128);
    audio.service();
    let deadline = Instant::now() + Duration::from_secs(3);
    while audio.capture.ready.is_none() && Instant::now() < deadline {
        audio.service();
        std::thread::sleep(Duration::from_millis(1));
    }
    let receipt = audio
        .take_hosted_capture_result(request)
        .expect("capture completed")
        .unwrap();
    assert!(
        audio.capture.resume_error.is_none(),
        "{:?}",
        audio.capture.resume_error
    );
    assert_ne!(
        audio.hosted_control.as_ref().unwrap().fence().incarnation,
        old_incarnation
    );
    let captured = session
        .complete_hosted_project_capture(&ticket, receipt)
        .unwrap();
    assert_eq!(captured.snapshot.frame, Some(device_cut));
    assert_eq!(captured.snapshot.state_source, "live_hosted_barrier");
    assert_eq!(
        json!(session.project_document()),
        before,
        "capture never loads its clone into history"
    );
    assert_eq!(session.history_state_ids().collect::<Vec<_>>(), history);
    assert_eq!(
        session.project_save_baseline().status_from_content(None),
        baseline
    );
    let captured_native = captured.snapshot.document.state_carrier().unwrap().patch;
    assert_eq!(captured_native.nodes[1].params["level"], 0.75);
    assert_eq!(captured_native.nodes[2].params["holding"], 1.);
    let saved = work.path().join("capture.json");
    std::fs::write(&saved, captured.snapshot.document.encode().unwrap()).unwrap();
    session
        .project_save_baseline()
        .register(&captured.checkpoint, Some(saved.to_string_lossy().into()))
        .unwrap();
    for _ in 0..8 {
        render(&mut callback, 128);
        audio.service();
    }
    assert_eq!(
        native_level(&callback),
        0.75,
        "resume preserves effective native control"
    );
    let hold = callback
        .graphs
        .active
        .engine
        .resolve_module_state("hold")
        .unwrap();
    assert_eq!(
        callback.graphs.active.engine.module_saved_state(hold),
        Some(SavedModuleState::PushHold(true))
    );
    assert_eq!(session.project_save_status()["dirty"], false);
    let mut reopened = Session::new(work.path()).unwrap();
    reopened
        .replace_project_document(captured.snapshot.document.clone(), None)
        .unwrap();
    assert_eq!(
        json!(reopened.project_document()),
        json!(&captured.snapshot.document)
    );
    reopened.request_delayed_hosted_execution(None).unwrap();
    let restored = reopened.audio_snapshot().unwrap();
    let prepared = audio.hosted_controller.prepare(&restored).unwrap();
    let restored_level = prepared.engine.resolve_parameter("amp", "level").unwrap();
    assert_eq!(prepared.engine.parameter_value(restored_level), Some(0.75));
    audio.dispose_graph({
        let mut graph = PreparedGraph::from(prepared.engine);
        graph.hosted = Some(prepared.runner);
        graph
    });
    let report = json!({"passed":true,"scope":"actual installed plugin, whole native graph capture, shared Session completion; paced/no device/window",
        "sealed_commands":130,"input_cut":frozen_input,"device_cut":device_cut,"native_level":0.75,"native_hold":true,
        "native_frozen_under_full_return_queue":true,"save_reopen":true,"resumed":true,
        "callback_allocations":0,"callback_deallocations":0,"device_qualified":false});
    drop(callback);
    drop(audio);
    assert!(std::fs::read_dir(children.path()).unwrap().next().is_none());
    std::fs::write(
        destination.join("coherent-capture-result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires explicitly supplied real ARM companion, SDK and installed plugin; no device opened"]
fn actual_application_capture_timeout_race_and_stale_edit_preserve_native_dsp() {
    let destination = std::path::PathBuf::from(
        std::env::var_os("OPENPATCH_HOSTED_CAPTURE_REVIEW").expect("review path"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    for mode in ["timeout", "stale", "dead"] {
        let children = tempfile::tempdir_in(&destination).unwrap();
        let work = tempfile::tempdir_in(&destination).unwrap();
        let mut session = Session::new(work.path()).unwrap();
        session.replace_project_document(document(), None).unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
        let native = session.project_document().state_carrier().unwrap().patch;
        let (mut audio, mut callback) = super::super::tests::host_test_audio(&native);
        audio
            .hosted_controller
            .set_config(real_configuration(children.path()));
        session
            .set_live_module_events(Some(audio.performance.clone()))
            .unwrap();
        submit(&mut audio, &mut session);
        for _ in 0..8 {
            render(&mut callback, 128);
            audio.service();
        }
        let old_process = audio.hosted_controller.statuses()[0].process_id;
        let level = callback
            .graphs
            .active
            .engine
            .resolve_parameter("amp", "level")
            .unwrap();
        assert!(callback
            .graphs
            .active
            .engine
            .set_parameter_value(level, 0.75));
        let ticket = session.begin_hosted_project_capture(None).unwrap();
        let request = audio.begin_hosted_capture(ticket.clone()).unwrap();
        render(&mut callback, 128); // swap at input1024, leaving the entire callback silent
        let baseline = session.project_save_baseline().status_from_content(None);
        if mode == "timeout" {
            // Model callback ownership between its cut check and publishing the
            // return; cancellation must retain a tombstone until it arrives.
            let packet = audio.capture.returns.pop().unwrap();
            audio.capture.pending.as_mut().unwrap().started =
                Instant::now() - CAPTURE_TIMEOUT - Duration::from_millis(1);
            audio.service();
            assert!(audio
                .take_hosted_capture_result(request)
                .unwrap()
                .err()
                .unwrap()
                .contains("timed out"));
            assert!(audio
                .capture
                .pending
                .as_ref()
                .unwrap()
                .cancel_error
                .is_some());
            assert!(callback.capture.returns.push(packet).is_ok());
            audio.service();
            render(&mut callback, 128);
            audio.service();
            assert_eq!(
                audio.hosted_controller.statuses()[0].process_id,
                old_process,
                "late canceled return restores the exact runner without restart"
            );
        } else if mode == "stale" {
            session
                .execute(
                    "set_parameter",
                    json!({"id":"source","parameter":"value","value":0.75}),
                )
                .unwrap();
            let current = session.audio_snapshot().unwrap();
            assert!(audio
                .submit_document(current.clone())
                .unwrap_err()
                .contains("capture"));
            audio.service();
            let deadline = Instant::now() + Duration::from_secs(2);
            while audio.capture.ready.is_none() && Instant::now() < deadline {
                audio.service();
                std::thread::sleep(Duration::from_millis(1));
            }
            let receipt = audio.take_hosted_capture_result(request).unwrap().unwrap();
            assert!(session
                .complete_hosted_project_capture(&ticket, receipt)
                .is_err());
            assert!(
                audio.capture.recovery.is_some(),
                "retain frozen native DSP for the newer ticket"
            );
            audio.submit_document(current.clone()).unwrap();
            assert!(session.acknowledge_audio_snapshot(current.id));
            for _ in 0..8 {
                render(&mut callback, 128);
                audio.service();
            }
            assert_eq!(audio.accepted_patch.nodes[0].params["value"], 0.75);
        } else {
            assert!(std::process::Command::new("/bin/kill")
                .args(["-KILL", &old_process.to_string()])
                .status()
                .unwrap()
                .success());
            let deadline = Instant::now() + Duration::from_secs(2);
            while audio.hosted_controller.statuses()[0].health_error.is_none()
                && Instant::now() < deadline
            {
                let _ = audio.hosted_controller.service();
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(audio.hosted_controller.statuses()[0].health_error.is_some());
            audio.service();
            assert!(
                audio.take_hosted_capture_result(request).unwrap().is_err(),
                "dead child cannot produce a successful save receipt"
            );
            assert_eq!(
                audio.hosted_controller.owner_count(),
                1,
                "resume failed: {:?}",
                audio.capture.resume_error
            );
            assert_ne!(
                audio.hosted_controller.statuses()[0].process_id,
                old_process
            );
            for _ in 0..8 {
                render(&mut callback, 128);
                audio.service();
            }
        }
        assert_eq!(
            native_level(&callback),
            0.75,
            "unrelated effective native control survives"
        );
        assert_eq!(
            session.project_save_baseline().status_from_content(None),
            baseline
        );
        drop(callback);
        drop(audio);
        assert!(std::fs::read_dir(children.path()).unwrap().next().is_none());
    }
    std::fs::write(destination.join("capture-cancellation-result.json"),serde_json::to_vec_pretty(&json!({
        "passed":true,"late_timeout_restores_same_runner":true,"stale_completion_refused":true,
        "native_dsp_preserved_for_newer_intent":true,"baseline_unchanged":true,"device_qualified":false
        ,"dead_child_capture_refused_and_native_resumed":true
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "requires explicit actual ARM companion/SDK/plugin; paced callback, no audio device"]
fn actual_public_hosted_controls_scope_native_continuity_and_mcp_save() {
    let destination = std::path::PathBuf::from(
        std::env::var_os("OPENPATCH_HOSTED_CAPTURE_REVIEW").expect("review path"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    let children = tempfile::tempdir_in(&destination).unwrap();
    let workspace = tempfile::tempdir_in(&destination).unwrap();
    let mut session = Session::new(workspace.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    session
        .execute(
            "set_hosted_execution",
            json!({"enabled":true,"expected_revision":session.revision()}),
        )
        .unwrap();
    let native = session.project_document().state_carrier().unwrap().patch;
    let (mut audio, mut callback) = super::super::tests::host_test_audio(&native);
    audio
        .hosted_controller
        .set_config(real_configuration(children.path()));
    session
        .set_live_module_events(Some(audio.performance.clone()))
        .unwrap();
    submit(&mut audio, &mut session);
    audio.set_playing(true);
    for _ in 0..8 {
        render(&mut callback, 128);
        audio.service();
    }
    let owner = audio.hosted_controller.statuses()[0].process_id;
    let incarnation = audio.hosted_control.as_ref().unwrap().fence().incarnation;
    let hold = callback
        .graphs
        .active
        .engine
        .resolve_module_state("hold")
        .unwrap();
    assert!(callback
        .graphs
        .active
        .engine
        .set_module_saved_state(hold, SavedModuleState::PushHold(true)));
    let mut frame = callback.graphs.active.hosted.as_ref().unwrap().frame();
    for level in [0.5, 0.75] {
        session
            .execute(
                "set_parameter",
                json!({"id":"amp","parameter":"level","value":level}),
            )
            .unwrap();
        submit(&mut audio, &mut session);
        render(&mut callback, 128);
        audio.service();
        frame += 128;
        assert_eq!(
            callback.graphs.active.hosted.as_ref().unwrap().frame(),
            frame
        );
        assert_eq!(native_level(&callback), level);
        assert_eq!(audio.hosted_controller.owner_count(), 1);
        assert_eq!(audio.hosted_controller.statuses()[0].process_id, owner);
        for _ in 0..4 {
            render(&mut callback, 128);
            audio.service();
            frame += 128;
        }
    }
    let generation = audio
        .set_scope(
            &native,
            Some(ScopeSelection {
                x: Some(ScopeEndpoint {
                    id: "amp".into(),
                    port: "out".into(),
                    channel: 0,
                }),
                y: None,
            }),
        )
        .unwrap();
    for _ in 0..5 {
        render(&mut callback, 128);
        audio.service();
        frame += 128;
    }
    assert_eq!(
        callback.graphs.active.hosted.as_ref().unwrap().frame(),
        frame
    );
    assert_eq!(
        audio.hosted_control.as_ref().unwrap().fence().incarnation,
        incarnation
    );
    assert_eq!(audio.hosted_controller.statuses()[0].process_id, owner);
    let packets: Vec<_> = audio.scope_packets().collect();
    assert!(packets
        .iter()
        .any(|packet| packet.generation == generation
            && packet.x_active.iter().any(|active| *active)));
    let hold = callback
        .graphs
        .active
        .engine
        .resolve_module_state("hold")
        .unwrap();
    assert_eq!(
        callback.graphs.active.engine.module_saved_state(hold),
        Some(SavedModuleState::PushHold(true))
    );
    // Native and MCP edit exactly the same history and bounded event channel.
    session.execute("set_hosted_parameter",json!({"id":"binary","parameter":"offset","value":0.25,"history_group":"public-gesture"})).unwrap();
    session.execute("set_hosted_parameter",json!({"id":"binary","parameter":"offset","value":0.5,"history_group":"public-gesture"})).unwrap();
    session.execute("undo", json!({})).unwrap();
    session.execute("redo", json!({})).unwrap();
    submit(&mut audio, &mut session);
    for _ in 0..8 {
        render(&mut callback, 128);
        audio.service();
    }
    let shared = Arc::new(std::sync::Mutex::new(session));
    let worker_session = shared.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = openpatch::control::execute_shared(
            &worker_session,
            "save_document",
            json!({"filename":"public-hosted.json"}),
        );
        send.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        audio.service();
        // The MCP worker must release Session while the real graph drains.
        let lock_start = Instant::now();
        let revision = shared.lock().unwrap().revision();
        assert!(revision > 0 && lock_start.elapsed() < Duration::from_secs(1));
        render(&mut callback, 128);
        audio.service();
        if let Ok(result) = receive.try_recv() {
            break result.unwrap();
        }
        assert!(Instant::now() < deadline, "MCP save did not complete");
    };
    worker.join().unwrap();
    assert_eq!(result["state_source"], "live_hosted_barrier");
    assert_eq!(result["affects_active_project_baseline"], false);
    let saved = ProjectDocument::decode(
        &std::fs::read(workspace.path().join("public-hosted.json")).unwrap(),
    )
    .unwrap();
    let saved_native = saved.state_carrier().unwrap().patch;
    assert_eq!(
        saved_native
            .nodes
            .iter()
            .find(|node| node.id == "amp")
            .unwrap()
            .params["level"],
        0.75
    );
    assert_eq!(
        saved_native
            .nodes
            .iter()
            .find(|node| node.id == "hold")
            .unwrap()
            .params["holding"],
        1.
    );
    let saved_hosted = saved
        .graph
        .nodes
        .iter()
        .find_map(|node| {
            if let DocumentNode::Hosted { module, .. } = node {
                Some(module)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(saved_hosted.parameter("offset").unwrap(), 0.5);
    assert!(saved_hosted.option("offset_first").unwrap());
    assert!(!saved_hosted.option("disable_output_limit").unwrap());
    for _ in 0..8 {
        render(&mut callback, 128);
        audio.service();
    }
    assert!(audio.performance.execution_available());
    shared
        .lock()
        .unwrap()
        .execute("set_hosted_execution", json!({"enabled":false}))
        .unwrap();
    drop(callback);
    drop(audio);
    drop(shared);
    assert!(std::fs::read_dir(children.path()).unwrap().next().is_none());
    std::fs::write(destination.join("public-hosted-workflow-result.json"),serde_json::to_vec_pretty(&json!({
        "passed":true,"scope":"actual installed binary with public Session activation, native/Scope graph continuity, grouped hosted edit/history and MCP save dispatch; paced/no device/window",
        "same_child_across_native_edits_and_scope":true,"scope_generation":generation,
        "mcp_save_state_source":result["state_source"],"callback_heap_operations":0,"device_qualified":false
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "requires explicit real ARM companion/SDK/plugin paths; paced callback, no device"]
fn actual_xfade_application_three_controls_native_continuity_and_coherent_save() {
    let destination=std::path::PathBuf::from(std::env::var_os("OPENPATCH_HOSTED_CAPTURE_REVIEW").expect("review path"));
    std::fs::create_dir_all(&destination).unwrap();
    let children=tempfile::tempdir_in(&destination).unwrap();let workspace=tempfile::tempdir_in(&destination).unwrap();
    let mut doc=document();
    for node in &mut doc.graph.nodes { if let DocumentNode::Hosted{module,..}=node {
        let mut identity=module.identity.clone();identity.model="Bogaudio-XFade".into();
        *module=HostedModule::xfade(identity,-1.,0.5,true).unwrap();
    }}
    for cable in &mut doc.graph.cables {if cable.route.to=="binary" {cable.route.input="a".into();}}
    let mut session=Session::new(workspace.path()).unwrap();session.replace_project_document(doc,None).unwrap();
    session.execute("set_hosted_execution",json!({"enabled":true,"expected_revision":session.revision()})).unwrap();
    let native=session.project_document().state_carrier().unwrap().patch;
    let (mut audio,mut callback)=super::super::tests::host_test_audio(&native);
    audio.hosted_controller.set_config(real_configuration(children.path()));session.set_live_module_events(Some(audio.performance.clone())).unwrap();
    submit(&mut audio,&mut session);audio.set_playing(true);
    for _ in 0..8 {render(&mut callback,128);audio.service();}
    let owner=audio.hosted_controller.statuses()[0].process_id;
    for (parameter,value) in [("mix",0.25),("mix",0.5),("curve",0.75),("linear",0.)] {
        session.execute("set_hosted_parameter",json!({"id":"binary","parameter":parameter,"value":value})).unwrap();
    }
    session.execute("undo",json!({})).unwrap();session.execute("redo",json!({})).unwrap();submit(&mut audio,&mut session);
    for _ in 0..10 {render(&mut callback,128);audio.service();}
    assert_eq!(audio.hosted_controller.statuses()[0].process_id,owner);
    let control=audio.hosted_control.as_ref().unwrap();assert_eq!(control.snapshot().applied.unwrap().values.values,[0.5,0.75,0.]);
    session.execute("set_parameter",json!({"id":"amp","parameter":"level","value":0.25})).unwrap();submit(&mut audio,&mut session);
    for _ in 0..8 {render(&mut callback,128);audio.service();}
    assert_eq!(audio.hosted_controller.statuses()[0].process_id,owner);assert_eq!(native_level(&callback),0.25);
    let shared=Arc::new(std::sync::Mutex::new(session));let worker_session=shared.clone();let(send,receive)=std::sync::mpsc::channel();
    let worker=std::thread::spawn(move||{send.send(openpatch::control::execute_shared(&worker_session,"save_document",json!({"filename":"xfade-live.json"}))).unwrap();});
    let deadline=Instant::now()+Duration::from_secs(10);
    let result=loop {audio.service();render(&mut callback,128);audio.service();if let Ok(result)=receive.try_recv(){break result.unwrap();}assert!(Instant::now()<deadline,"XFade save barrier timed out");};
    worker.join().unwrap();assert_eq!(result["state_source"],"live_hosted_barrier");
    let saved=ProjectDocument::decode(&std::fs::read(workspace.path().join("xfade-live.json")).unwrap()).unwrap();
    let module=saved.graph.nodes.iter().find_map(|n|match n{DocumentNode::Hosted{module,..}=>Some(module),_=>None}).unwrap();
    module.validate_supported().unwrap();for(key,value)in[("mix",0.5),("curve",0.75),("linear",0.)]{assert_eq!(module.parameter(key).unwrap(),value);}
    assert_eq!(module.state.as_ref()["data"],json!({}));
    drop(callback);drop(audio);assert!(std::fs::read_dir(children.path()).unwrap().next().is_none());
    std::fs::write(destination.join("xfade-application-result.json"),serde_json::to_vec_pretty(&json!({"passed":true,"same_child_for_three_controls_history_and_native_edit":true,"coherent_save":true,"parent_callback_heap_activity":0,"device_qualified":false})).unwrap()).unwrap();
}
