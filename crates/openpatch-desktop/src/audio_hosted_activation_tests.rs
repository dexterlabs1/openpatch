mod app_host_activation {
    use super::*;
    use openpatch::project_document::DocumentNode;
    use std::time::{Duration, Instant};

    fn constant_document() -> openpatch::project_document::ProjectDocument {
        let (_, mut document) = hosted_document();
        let DocumentNode::Native { node, .. } = &mut document.graph.nodes[0] else {
            panic!("missing source")
        };
        node.kind = "constant".into();
        node.params.insert("value".into(), 0.5);
        document.graph.cables[0].route.output = "out".into();
        document
    }

    fn session() -> (tempfile::TempDir, openpatch::control::Session) {
        let directory = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(directory.path()).unwrap();
        session
            .replace_project_document(constant_document(), None)
            .unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
        (directory, session)
    }

    #[test]
    fn authored_receipt_is_specific_to_its_graph_and_never_rolls_back() {
        let patch: Patch = serde_json::from_value(serde_json::json!({
            "nodes":[{"id":"source","kind":"constant","params":{"value":1.}}]
        }))
        .unwrap();
        let mut active = PreparedGraph::from(patch.compile().unwrap());
        let mut pending = PreparedGraph::from(patch.compile().unwrap());
        let old = Arc::new(AtomicU64::new(4));
        let new = Arc::new(AtomicU64::new(7));
        active.authored_revision = Some(old.clone());
        pending.authored_revision = Some(new.clone());
        new.store(8, Ordering::Release);
        assert_eq!(active.current_revision(), 4);
        assert_eq!(pending.current_revision(), 8);
        old.store(6, Ordering::Release);
        assert_eq!(pending.current_revision(), 8);
        assert_eq!(active.current_revision(), 6);
    }

    fn configuration(destination: &std::path::Path) -> hosted_audio::HostedAudioConfig {
        let path = |key| std::path::PathBuf::from(std::env::var_os(key).expect(key));
        hosted_audio::HostedAudioConfig {
            companion_executable: path("OPENPATCH_HOSTED_COMPANION"),
            sdk_dir: path("OPENPATCH_HOSTED_SDK"),
            plugin_dir: path("OPENPATCH_HOSTED_PLUGIN"),
            workspace_parent: destination.into(),
        }
    }

    // These checks deliberately pace the child, so they prove application
    // ownership/authority and exact samples, not continuous device deadlines.
    fn render_blocks(
        audio: &mut Audio,
        callback: &mut Callback,
        samples: &mut Vec<f32>,
        blocks: usize,
    ) {
        for _ in 0..blocks {
            std::thread::sleep(Duration::from_millis(5));
            let mut output = [0.0; 128];
            without_heap_activity(|| callback.render(&mut output, 1));
            samples.extend_from_slice(&output);
            audio.service();
        }
    }

    fn submit_current(audio: &mut Audio, session: &mut openpatch::control::Session) {
        let ticket = session.audio_snapshot().unwrap();
        audio.submit_document(ticket.clone()).unwrap();
        assert!(session.acknowledge_audio_snapshot(ticket.id));
    }

    #[test]
    #[ignore = "requires explicitly supplied real ARM companion, SDK and installed plugin; no device opened"]
    fn actual_application_submit_edit_history_retirement_and_shutdown() {
        let destination = std::path::PathBuf::from(
            std::env::var_os("OPENPATCH_HOSTED_ACTIVATION_REVIEW").expect("review destination"),
        );
        std::fs::create_dir_all(&destination).unwrap();
        let workspace = tempfile::tempdir_in(&destination).unwrap();
        let (_directory, mut session) = session();
        let ticket = session.audio_snapshot().unwrap();
        let native = ticket.document.state_carrier().unwrap().patch;
        let (mut audio, mut callback) = host_test_audio(&native);
        audio
            .hosted_controller
            .set_config(configuration(workspace.path()));
        session
            .set_live_module_events(Some(audio.performance.clone()))
            .unwrap();
        audio.submit_document(ticket.clone()).unwrap();
        assert!(session.acknowledge_audio_snapshot(ticket.id));
        assert!(
            !audio.performance.execution_available(),
            "preparation is not activation"
        );
        let incarnation = audio.hosted_control.as_ref().unwrap().fence().incarnation;
        let owner = audio.hosted_controller.statuses()[0].process_id;
        let requested = audio.hosted_runtime_status();
        audio.set_playing(true);
        audio
            .shared
            .volume
            .store(1.0f32.to_bits(), Ordering::Relaxed);
        callback.gain.value = 1.;
        let mut samples = Vec::new();
        render_blocks(&mut audio, &mut callback, &mut samples, 8);
        assert!(audio.performance.execution_available());
        let mut operations = Vec::new();
        for (tool, args) in [
            (
                "set_hosted_parameter",
                serde_json::json!({"id":"binary","parameter":"offset","value":0.25}),
            ),
            ("undo", serde_json::json!({})),
            ("redo", serde_json::json!({})),
            (
                "set_hosted_parameter",
                serde_json::json!({"id":"binary","parameter":"scale","value":-0.25}),
            ),
        ] {
            let input_frame = samples.len();
            session.execute(tool, args).unwrap();
            submit_current(&mut audio, &mut session);
            assert_eq!(audio.hosted_controller.owner_count(), 1);
            assert_eq!(audio.hosted_controller.statuses()[0].process_id, owner);
            assert_eq!(
                audio.hosted_control.as_ref().unwrap().fence().incarnation,
                incarnation
            );
            render_blocks(&mut audio, &mut callback, &mut samples, 8);
            operations.push(serde_json::json!({"tool":tool,"input_frame":input_frame,"revision":audio.submitted_revision}));
            assert_eq!(callback.graphs.active.revision, audio.submitted_revision);
            assert!(audio.performance.execution_available());
        }
        let expected: Vec<f32> = (0..samples.len())
            .map(|frame| match frame {
                0..387 => 0.,
                387..1409 => 0.0390625,
                1409..2433 => 0.234375,
                2433..3457 => 0.0390625,
                3457..4481 => 0.234375,
                _ => -0.234375,
            })
            .collect();
        let mismatch = samples.iter().zip(&expected).enumerate().find(|(_, (a,b))| a.to_bits() != b.to_bits())
            .map(|(frame,(actual,expected))| serde_json::json!({"frame":frame,"actual":actual,"expected":expected}));
        let applied = audio.hosted_control.as_ref().unwrap().snapshot();
        let performance = audio.performance.snapshot();
        let baseline = session.project_save_status();
        assert!(session
            .capture_active_project_save(None)
            .err()
            .unwrap()
            .contains("save barrier"));
        assert_eq!(
            session.project_save_status(),
            baseline,
            "a refused live capture cannot acknowledge a saved baseline"
        );
        // Stop the callback and return its exact endpoint on the control thread.
        let graph = std::mem::replace(
            &mut callback.graphs.active,
            native.compile().unwrap().into(),
        );
        audio.dispose_graph(graph);
        let deadline = Instant::now() + Duration::from_secs(3);
        while audio.hosted_controller.owner_count() != 0 && Instant::now() < deadline {
            audio.service();
            std::thread::sleep(Duration::from_millis(1));
        }
        let retired = audio.hosted_runtime_status();
        let capture = audio
            .last_hosted_retirement
            .as_ref()
            .unwrap()
            .captured
            .clone();
        let authored = &session.project_document().graph.nodes;
        let authored_state = authored
            .iter()
            .find_map(|node| match node {
                DocumentNode::Hosted { module, .. } => Some(module.state.as_ref()),
                _ => None,
            })
            .unwrap();
        let capture_matches = capture.as_ref() == Some(authored_state);
        let no_owners = audio.hosted_controller.owner_count() == 0;
        drop(callback);
        drop(audio);
        let workspace_empty = std::fs::read_dir(workspace.path())
            .unwrap()
            .next()
            .is_none();
        let passed = mismatch.is_none()
            && capture_matches
            && no_owners
            && workspace_empty
            && applied.latest_accepted_sequence == 4
            && applied.outstanding_commands == 0;
        let report = serde_json::json!({"passed":passed,"scope":"actual Audio submit/service + desktop Callback + shared Session history; paced, no device/window",
            "frames":samples.len(),"rate":48000,"operations":operations,"first_mismatch":mismatch,
            "requested":requested,"applied":applied,"performance":performance,"retired":retired,
            "capture_matches_authored":capture_matches,"workspace_empty":workspace_empty,
            "callback_allocations":0,"callback_deallocations":0,"device_qualified":false});
        std::fs::write(
            destination.join("application-activation-result.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        assert!(passed, "{}", serde_json::to_string(&report).unwrap());
    }

    #[test]
    #[ignore = "requires explicitly supplied real ARM companion, SDK and installed plugin; no device opened"]
    fn actual_application_unadmitted_edits_and_pending_capture_are_not_acknowledged() {
        let destination = std::path::PathBuf::from(
            std::env::var_os("OPENPATCH_HOSTED_ACTIVATION_REVIEW").expect("review destination"),
        );
        std::fs::create_dir_all(&destination).unwrap();
        let workspace = tempfile::tempdir_in(&destination).unwrap();
        let (_directory, mut session) = session();
        let native = session.project_document().state_carrier().unwrap().patch;
        let (mut audio, mut callback) = host_test_audio(&native);
        audio
            .hosted_controller
            .set_config(configuration(workspace.path()));
        session
            .set_live_module_events(Some(audio.performance.clone()))
            .unwrap();
        submit_current(&mut audio, &mut session);
        audio.set_playing(true);
        audio
            .shared
            .volume
            .store(1.0f32.to_bits(), Ordering::Relaxed);
        callback.gain.value = 1.;
        let mut samples = Vec::new();
        render_blocks(&mut audio, &mut callback, &mut samples, 8);
        let old_incarnation = audio.hosted_control.as_ref().unwrap().fence().incarnation;
        session.set_live_module_events(None).unwrap();
        session
            .execute(
                "set_hosted_parameter",
                serde_json::json!({"id":"binary","parameter":"offset","value":0.25}),
            )
            .unwrap();
        submit_current(&mut audio, &mut session);
        assert_ne!(audio.hosted_control.as_ref().unwrap().fence().incarnation, old_incarnation,
            "an authored-only ticket must prepare its state, never falsely reuse an unadmitted command");
        render_blocks(&mut audio, &mut callback, &mut samples, 8);
        assert!(samples[samples.len() - 128..]
            .iter()
            .all(|sample| *sample == 0.234375));
        assert_eq!(audio.hosted_controller.owner_count(), 1);
        session
            .set_live_module_events(Some(audio.performance.clone()))
            .unwrap();
        session
            .execute(
                "set_hosted_parameter",
                serde_json::json!({"id":"binary","parameter":"scale","value":-0.25}),
            )
            .unwrap();
        assert_eq!(
            audio
                .hosted_control
                .as_ref()
                .unwrap()
                .snapshot()
                .outstanding_commands,
            1
        );
        let graph = std::mem::replace(
            &mut callback.graphs.active,
            native.compile().unwrap().into(),
        );
        audio.dispose_graph(graph);
        let receipt = audio.last_hosted_retirement.as_ref().unwrap();
        assert!(receipt.captured.is_none());
        assert!(receipt.error.as_ref().unwrap().contains("unacknowledged"));
        let report = audio.hosted_runtime_status();
        assert_eq!(audio.hosted_controller.owner_count(), 0);
        drop(callback);
        drop(audio);
        assert!(std::fs::read_dir(workspace.path())
            .unwrap()
            .next()
            .is_none());
        std::fs::write(
            destination.join("application-unadmitted-result.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }

    #[test]
    #[ignore = "requires explicitly supplied real ARM companion, SDK and installed plugin; no device opened"]
    fn actual_application_dead_child_and_queued_shutdown_release_all_owners() {
        let destination = std::path::PathBuf::from(
            std::env::var_os("OPENPATCH_HOSTED_ACTIVATION_REVIEW").expect("review destination"),
        );
        std::fs::create_dir_all(&destination).unwrap();
        let workspace = tempfile::tempdir_in(&destination).unwrap();
        let (_directory, mut session) = session();
        let native = session.project_document().state_carrier().unwrap().patch;
        let (mut audio, mut callback) = host_test_audio(&native);
        audio
            .hosted_controller
            .set_config(configuration(workspace.path()));
        session
            .set_live_module_events(Some(audio.performance.clone()))
            .unwrap();
        submit_current(&mut audio, &mut session);
        let status = audio.hosted_controller.statuses().pop().unwrap();
        assert!(std::process::Command::new("/bin/kill")
            .args(["-KILL", &status.process_id.to_string()])
            .status()
            .unwrap()
            .success());
        let deadline = Instant::now() + Duration::from_secs(2);
        while audio.hosted_controller.statuses()[0].health_error.is_none()
            && Instant::now() < deadline
        {
            audio.service();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(audio.hosted_controller.statuses()[0].health_error.is_some());
        assert!(!audio.performance.execution_available());
        assert_eq!(
            audio.hosted_controller.owner_count(),
            1,
            "owner stays until callback returns its endpoint"
        );
        let mut output = [1.; 128];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.; 128]);
        let report = audio.hosted_runtime_status();
        // The next request is still queued, and is destroyed before owner cleanup.
        drop(callback);
        drop(audio);
        assert!(std::fs::read_dir(workspace.path())
            .unwrap()
            .next()
            .is_none());

        // Also leave a healthy prepared child unread in the incoming ring.
        let (_directory, mut session) = self::session();
        let (mut audio, callback) = host_test_audio(&native);
        audio
            .hosted_controller
            .set_config(configuration(workspace.path()));
        submit_current(&mut audio, &mut session);
        assert_eq!(audio.hosted_controller.owner_count(), 1);
        drop(callback);
        drop(audio);
        assert!(std::fs::read_dir(workspace.path())
            .unwrap()
            .next()
            .is_none());
        std::fs::write(
            destination.join("application-failure-result.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
