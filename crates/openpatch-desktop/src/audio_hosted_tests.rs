mod hosted_callback {
    use super::*;
    use openpatch::binary_host::{runner::HostedRunner, Fence, Region};
    use openpatch_core::ExternalModuleSpec;
    use std::os::unix::fs::FileExt;

    fn hosted_document() -> (Patch, openpatch::project_document::ProjectDocument) {
        use openpatch::project_document::hosted::{HostedIdentity, HostedModule};
        use openpatch::project_document::{DocumentNode, ProjectDocument, ProjectLayout};
        let native:Patch=serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"push","kind":"push","params":{}},
                     {"id":"amp","kind":"gain","params":{"gain":0.125}}],"cables":[],"output":"amp"})).unwrap();
        let mut document =
            ProjectDocument::from_patch(native.clone(), ProjectLayout::default()).unwrap();
        document.graph.nodes.push(DocumentNode::Hosted {
            id: "binary".into(),
            source: None,
            module: HostedModule::offset(
                HostedIdentity {
                    plugin: "Bogaudio".into(),
                    model: "Bogaudio-Offset".into(),
                    version: "2.6.47".into(),
                    rack_version: "2.6.6".into(),
                    plugin_sha256:
                        "264e55e1389274c3873439372c5c51f48f63168bd6c98fb2c6ffec170224b6f6".into(),
                    runtime_sha256:
                        "55dd40f7dff09a6ed7b93374b150f33931034d3754c128da7b209ea37b6aaf1c".into(),
                },
                0.,
                0.25,
                true,
                false,
            )
            .unwrap(),
        });
        for route in [
            serde_json::json!({"id":"in","from":"push","output":"gate","to":"binary","input":"in"}),
            serde_json::json!({"id":"out","from":"binary","output":"out","to":"amp","input":"signal"}),
        ] {
            document
                .graph
                .cables
                .push(serde_json::from_value(serde_json::json!({"route":route})).unwrap());
        }
        (native, document)
    }

    #[test]
    fn hosted_document_request_cannot_fall_back_to_native_on_submit_or_device_restart() {
        let (patch, document) = hosted_document();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_project_document(document, None).unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
        let ticket = session.audio_snapshot().unwrap();
        let (mut audio, _callback) = host_test_audio(&patch);
        let revision = audio.submitted_revision;
        assert!(audio
            .submit_document(ticket.clone())
            .unwrap_err()
            .contains("OPENPATCH_HOSTED_"));
        assert_eq!(audio.submitted_revision, revision);
        assert!(audio.pending.is_none());
        assert!(audio.accepted_document.is_none());
        let error =
            Audio::new_with_document(ticket, Some("no audio device may be consulted"), false)
                .err()
                .unwrap();
        assert!(error.contains("OPENPATCH_HOSTED_"));
    }

    #[test]
    fn full_ticket_enforces_suspension_without_an_attached_session_backend() {
        let (patch, document) = hosted_document();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_project_document(document, None).unwrap();
        let suspended = session.audio_snapshot().unwrap();
        let (mut audio, mut callback) = host_test_audio(&patch);
        assert!(audio.performance.execution_available());
        audio.submit_document(suspended.clone()).unwrap();
        assert!(!audio.performance.execution_available());
        audio.set_playing(true);
        let mut samples = [1.0_f32; 128];
        callback.render(&mut samples, 1);
        assert_eq!(samples, [0.; 128]);
        assert!(!audio.performance.execution_available());
        assert!(session.acknowledge_audio_snapshot(suspended.id));
        session
            .set_document_playback(openpatch::project_document::PlaybackMode::Partial, None)
            .unwrap();
        audio
            .submit_document(session.audio_snapshot().unwrap())
            .unwrap();
        callback.render(&mut samples, 1);
        assert!(
            audio.performance.execution_available(),
            "only explicit Partial authorizes the omitted native graph"
        );
    }

    #[test]
    fn performance_keeps_one_hosted_event_binding_for_shared_edits_and_history() {
        use openpatch::binary_host::control::HostedControl;
        use openpatch::control::DocumentExecutionRequest;
        let (patch, document) = hosted_document();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_project_document(document, None).unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
        let ticket = session.audio_snapshot().unwrap();
        let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
            panic!("missing grant")
        };
        let (control, mut reader) = HostedControl::new(
            grant.clone(),
            &ticket.document,
            Fence {
                incarnation: 991,
                generation: ticket.document_generation,
                revision: 77,
            },
            ticket.revision,
        )
        .unwrap();
        let (audio, _callback) = host_test_audio(&patch);
        audio
            .performance
            .register_hosted_control(&ticket, control.clone())
            .unwrap();
        session
            .set_live_module_events(Some(audio.performance.clone()))
            .unwrap();
        session
            .execute(
                "set_hosted_parameter",
                serde_json::json!({"id":"binary","parameter":"scale","value":-0.25}),
            )
            .unwrap();
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("redo", serde_json::json!({})).unwrap();
        let inspection = audio.performance.snapshot();
        assert_eq!(inspection["hosted_controls"]["incarnation"], 991);
        assert_eq!(
            inspection["hosted_controls"]["plan_epoch"], 77,
            "authored revisions must not restart the execution epoch"
        );
        assert_eq!(inspection["hosted_controls"]["latest_accepted_sequence"], 3);
        assert_eq!(inspection["hosted_controls"]["available"], false);
        audio
            .performance
            .sync_document(openpatch::control::DocumentExecutionContext {
                document: &ticket.document,
                document_generation: ticket.document_generation,
                request: DocumentExecutionRequest::Native(
                    openpatch::project_document::ExecutionDisposition::SuspendedUnresolved,
                ),
                native_carrier: &patch,
                revision: ticket.revision,
                fence_native: true,
            });
        assert!(
            !control.snapshot().closed,
            "an older context cannot close a newer binding"
        );
        assert!(
            audio
                .performance
                .register_hosted_control(&ticket, control.clone())
                .is_err(),
            "an old ticket cannot replace the newer publication floor"
        );
        for (sequence, value) in [(1, -0.25), (2, 0.25), (3, -0.25)] {
            let event = reader.next_for_sample(0).unwrap().unwrap();
            assert_eq!(
                (event.sequence, event.parameter, event.value),
                (sequence, 1, value)
            );
        }
        assert!(reader.next_for_sample(0).unwrap().is_none());
        assert!(
            control.snapshot().applied.is_none(),
            "scheduling alone is not application"
        );
        // A custom option needs a fresh plan. Full document synchronization
        // invalidates the reader; it cannot keep executing the old incarnation.
        session
            .set_hosted_option("binary", "offset_first", false, None)
            .unwrap();
        assert!(control.snapshot().closed);
        assert!(reader.next_for_sample(1).is_err());
        assert!(audio.performance.snapshot()["hosted_controls"].is_null());
        audio.performance.close();
        assert!(audio
            .performance
            .register_hosted_control(&ticket, control)
            .unwrap_err()
            .contains("closed"));
    }

    #[test]
    #[ignore = "requires explicitly supplied real ARM companion, SDK and installed plugin; no device opened"]
    fn hosted_callback_actual_plugin_native_impulse_and_control_capture() {
        use openpatch::binary_host::{graph::HostedGraphPlan, process::LaunchConfig, OutputBlock};
        use std::time::{Duration, Instant};
        let path = |key| std::path::PathBuf::from(std::env::var_os(key).expect(key));
        let destination = path("OPENPATCH_HOSTED_CALLBACK_REVIEW");
        std::fs::create_dir_all(&destination).unwrap();
        let (native, document) = hosted_document();
        let plan = HostedGraphPlan::prepare(&document).unwrap();
        let banks = native
            .prepare_wave_banks(&WavePreparationCache::default())
            .unwrap();
        let (engine, handle) = plan.compile(&banks).unwrap();
        let prepared = LaunchConfig {
            companion_executable: path("OPENPATCH_HOSTED_COMPANION"),
            sdk_dir: path("OPENPATCH_HOSTED_SDK"),
            plugin_dir: path("OPENPATCH_HOSTED_PLUGIN"),
            workspace_parent: destination.clone(),
            expected_plugin_sha256: plan.module().identity.plugin_sha256.clone(),
            expected_runtime_sha256: plan.module().identity.runtime_sha256.clone(),
            fence: Fence {
                incarnation: 30,
                generation: 2,
                revision: 1,
            },
            input_mask: plan.input_mask(),
            snapshot: plan.module().state.as_ref().clone(),
            startup_timeout: Duration::from_secs(5),
        }
        .launch()
        .unwrap();
        let process = prepared.process;
        std::fs::copy(
            process.workspace_path().join("private-user/identity.json"),
            destination.join("desktop-callback-identity.json"),
        )
        .unwrap();
        let hosted = HostedRunner::prepare(prepared.endpoint, handle).unwrap();
        let (_updates, incoming) = RingBuffer::new(2);
        let (retired, _disposal) = RingBuffer::new(2);
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::Relaxed);
        shared.volume.store(1f32.to_bits(), Ordering::Relaxed);
        let mut callback = Callback::new(engine, incoming, retired, shared, 48000);
        callback.performance.sync_patch(&native, 1);
        callback.graphs.active.modules = callback
            .performance
            .plan(&native, &callback.graphs.active.engine, 1)
            .unwrap();
        callback.graphs.active.revision = 1;
        callback.graphs.active.hosted = Some(hosted);
        callback.gain.value = 1.;
        let mut samples = vec![0f32; 2048];
        let mut timings = Vec::new();
        let origin = Instant::now() + Duration::from_millis(2);
        let mut frame = 0;
        let mut index = 0;
        while frame < samples.len() {
            let length = if index < 2 {
                1
            } else if index == 2 {
                126
            } else {
                128
            }
            .min(samples.len() - frame);
            let target = origin + Duration::from_secs_f64(frame as f64 / 48000.);
            if let Some(delay) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            }
            if frame == 0 {
                callback
                    .performance
                    .trigger("push", openpatch_core::ModuleAction::PushPress)
                    .unwrap();
            }
            if frame == 1 {
                callback
                    .performance
                    .trigger("push", openpatch_core::ModuleAction::PushRelease)
                    .unwrap();
            }
            let started = Instant::now();
            without_heap_activity(|| callback.render(&mut samples[frame..frame + length], 1));
            timings.push(serde_json::json!({"frame":frame,"frames":length,"duration_ns":started.elapsed().as_nanos(),"lateness_ns":started.saturating_duration_since(target).as_nanos()}));
            frame += length;
            index += 1;
        }
        let status = callback.performance.snapshot();
        let (mut endpoint, tail, incomplete, fault) =
            callback.graphs.active.hosted.take().unwrap().retire();
        let submitted = endpoint.submitted_blocks();
        let received = endpoint.received_blocks();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut scratch = OutputBlock::default();
        let mut drain_error = None;
        while !endpoint.is_drained() && fault.is_none() {
            match endpoint.try_receive(&mut scratch) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if Instant::now() >= deadline {
                        drain_error = Some("control drain timeout".to_string());
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(error) => {
                    drain_error = Some(format!("{error:?}"));
                    break;
                }
            }
        }
        let capture = if fault.is_none() && drain_error.is_none() {
            process
                .finish(endpoint)
                .map(|capture| capture.state)
                .map_err(|e| e.to_string())
        } else {
            drop(endpoint);
            drop(process);
            Err("callback failed; capture refused".into())
        };
        let nonzero: Vec<_> = samples
            .iter()
            .enumerate()
            .filter(|(_, value)| **value != 0.)
            .map(|(frame, value)| serde_json::json!({"frame":frame,"value":value}))
            .collect();
        let correct = nonzero == vec![serde_json::json!({"frame":387,"value":0.78125})];
        let capture_matches = capture
            .as_ref()
            .is_ok_and(|state| state == plan.module().state.as_ref());
        let passed = correct
            && capture_matches
            && tail == 0
            && !incomplete
            && fault.is_none()
            && drain_error.is_none();
        let report = serde_json::json!({"passed":passed,"gate":"actual installed plugin through desktop callback; no native device or GUI activation",
            "rate":48000,"frames":samples.len(),"transport_delay_frames":384,"native_source_to_native_receiver_frames":387,
            "nonzero":nonzero,"submitted":submitted,"received_before_control_drain":received,"unsubmitted_tail_frames":tail,"incomplete_frame":incomplete,
            "fault":format!("{fault:?}"),"drain_error":drain_error,"capture":capture,"capture_matches_authored_snapshot":capture_matches,"performance":status,"callbacks":timings,
            "callback_allocations":0,"callback_deallocations":0,"device_qualified":false,"automatic_activation_available":false});
        std::fs::write(
            destination.join("desktop-callback-result.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        openpatch_core::write_wav(&destination.join("desktop-callback.wav"), &samples, 48000)
            .unwrap();
        assert!(passed, "{}", serde_json::to_string(&report).unwrap());
    }

    fn callback(
        directory: &std::path::Path,
    ) -> (Callback, Producer<PreparedGraph>, Consumer<PreparedGraph>) {
        callback_with_rate(directory, 48000)
    }
    fn callback_with_rate(
        directory: &std::path::Path,
        stream_rate: u32,
    ) -> (Callback, Producer<PreparedGraph>, Consumer<PreparedGraph>) {
        let patch: Patch =
            serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}},
                     {"id":"amp","kind":"gain","params":{"gain":0.5}}],"cables":[],"output":"amp"}))
            .unwrap();
        let routes = serde_json::from_value::<Vec<openpatch_core::Cable>>(serde_json::json!([
            {"id":"input","from":"dc","output":"out","to":"binary","input":"in"},
            {"id":"output","from":"binary","output":"out","to":"amp","input":"signal"}]))
        .unwrap();
        let banks = patch
            .prepare_wave_banks(&WavePreparationCache::default())
            .unwrap();
        let engine = Engine::compile_with_externals(
            &patch,
            &[ExternalModuleSpec {
                id: "binary".into(),
                inputs: vec!["offset_cv".into(), "scale_cv".into(), "in".into()],
                outputs: vec!["out".into()],
            }],
            &routes,
            Some("amp"),
            "out",
            &banks,
        )
        .unwrap();
        let handle = engine.resolve_external_module("binary").unwrap();
        let path = directory.join("callback-transport");
        let region = Region::create(
            &path,
            Fence {
                incarnation: 7,
                generation: 3,
                revision: 1,
            },
            4,
            48000,
        )
        .unwrap();
        // Single-thread fixture publishes only the protocol's READY header; it
        // never simulates successful plugin output. The real companion gate is
        // separate. No peer file I/O runs inside the measured callback.
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .write_all_at(&1u32.to_le_bytes(), 68)
            .unwrap();
        let hosted = HostedRunner::prepare(region.into_endpoint().unwrap(), handle).unwrap();
        let (updates, incoming) = RingBuffer::new(2);
        let (retired, disposal) = RingBuffer::new(2);
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::Relaxed);
        shared.volume.store(1f32.to_bits(), Ordering::Relaxed);
        let mut callback = Callback::new(engine, incoming, retired, shared, stream_rate);
        callback.graphs.active.hosted = Some(hosted);
        callback.gain.value = 1.;
        (callback, updates, disposal)
    }

    #[test]
    fn hosted_callback_primes_then_latches_deadline_silence_and_reports_failure_without_heap() {
        let dir = tempfile::tempdir().unwrap();
        let (mut callback, _updates, _retired) = callback(dir.path());
        for _ in 0..3 {
            let mut output = [0.75f32; 128];
            without_heap_activity(|| callback.render(&mut output, 1));
            assert!(output.iter().all(|value| *value == 0.));
        }
        assert_eq!(
            callback.performance.snapshot()["hosted_runtime"]["status"],
            "priming"
        );
        let mut output = [0.75f32; 128];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert!(output.iter().all(|value| *value == 0.));
        assert_eq!(callback.frame_clock, 512);
        assert_eq!(callback.graphs.active.hosted.as_ref().unwrap().frame(), 384);
        let status = callback.performance.snapshot();
        assert_eq!(status["hosted_runtime"]["status"], "suspended_fault");
        assert_eq!(status["hosted_runtime"]["reason"], "missed_deadline");
        assert_eq!(status["available"], false);
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(callback.frame_clock, 640);
        assert_eq!(callback.graphs.active.hosted.as_ref().unwrap().frame(), 384);
    }

    #[test]
    fn hosted_callback_refuses_actual_oversized_buffer_before_internal_chunking() {
        let dir = tempfile::tempdir().unwrap();
        let (mut callback, _updates, _retired) = callback(dir.path());
        let mut output = [0.75f32; 256];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert!(output.iter().all(|value| *value == 0.));
        assert_eq!(
            callback
                .graphs
                .active
                .hosted
                .as_ref()
                .unwrap()
                .submitted_blocks(),
            0
        );
        assert_eq!(
            callback.performance.snapshot()["hosted_runtime"]["reason"],
            "unsupported_callback_profile"
        );
    }

    #[test]
    fn hosted_callback_refuses_wrong_actual_stream_rate_even_with_a_48000_hz_engine() {
        let dir = tempfile::tempdir().unwrap();
        let (mut callback, _updates, _retired) = callback_with_rate(dir.path(), 44100);
        assert_eq!(callback.graphs.active.engine.sample_rate(), 48000);
        let mut output = [0.75f32; 128];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert!(output.iter().all(|value| *value == 0.));
        assert_eq!(callback.graphs.active.hosted.as_ref().unwrap().frame(), 0);
        assert_eq!(
            callback
                .graphs
                .active
                .hosted
                .as_ref()
                .unwrap()
                .submitted_blocks(),
            0
        );
        assert_eq!(
            callback.performance.snapshot()["hosted_runtime"]["reason"],
            "unsupported_callback_profile"
        );
    }

    #[test]
    fn hosted_callback_retires_stale_hosted_plan_without_activating_or_dropping_it() {
        let first = tempfile::tempdir().unwrap();
        let (mut callback, mut updates, mut retired) = callback(first.path());
        callback.graphs.active.revision = 10;
        let second = tempfile::tempdir().unwrap();
        let (mut stale, _updates, _retired) = self::callback(second.path());
        let graph = std::mem::replace(
            &mut stale.graphs.active,
            Patch::new_v2().compile().unwrap().into(),
        );
        let mut graph = graph;
        graph.revision = 9;
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0f32; 32], 1));
        assert_eq!(callback.graphs.active.revision, 10);
        assert_eq!(callback.graphs.active.hosted.as_ref().unwrap().frame(), 32);
        let mut rejected = retired.pop().unwrap();
        assert_eq!(rejected.revision, 9);
        let (endpoint, tail, incomplete, fault) = rejected.hosted.take().unwrap().retire();
        assert_eq!(endpoint.submitted_blocks(), 0);
        assert_eq!(tail, 0);
        assert!(!incomplete);
        assert!(fault.is_none());
    }

    #[test]
    fn hosted_callback_freezes_donor_and_retires_endpoint_when_queue_was_full() {
        let dir = tempfile::tempdir().unwrap();
        let (mut callback, mut updates, mut retired) = callback(dir.path());
        without_heap_activity(|| callback.render(&mut [0f32; 96], 1));
        let native:Patch=serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"native","kind":"constant","params":{"value":0.25}}],"cables":[],"output":"native"})).unwrap();
        for _ in 0..2 {
            assert!(callback
                .graphs
                .retired
                .push(native.compile().unwrap().into())
                .is_ok());
        }
        assert!(updates.push(native.compile().unwrap().into()).is_ok());
        without_heap_activity(|| callback.render(&mut [0f32; 32], 1));
        assert!(callback.frozen_previous.is_some());
        assert_eq!(
            callback
                .graphs
                .previous
                .as_ref()
                .unwrap()
                .hosted
                .as_ref()
                .unwrap()
                .frame(),
            96
        );
        without_heap_activity(|| callback.render(&mut [0f32; 256], 1));
        assert!(callback.graphs.previous.is_none());
        let waiting = callback.graphs.waiting_retirement.as_ref().unwrap();
        assert_eq!(waiting.hosted.as_ref().unwrap().frame(), 96);
        assert_eq!(waiting.hosted.as_ref().unwrap().submitted_blocks(), 0);
        drop(retired.pop().unwrap());
        drop(retired.pop().unwrap());
        without_heap_activity(|| callback.render(&mut [0f32; 1], 1));
        let mut graph = retired.pop().unwrap();
        let (_endpoint, tail, incomplete, fault) = graph.hosted.take().unwrap().retire();
        assert_eq!(tail, 96);
        assert!(!incomplete);
        assert!(fault.is_none());
        assert_eq!(
            callback.performance.snapshot()["hosted_runtime"]["status"],
            "not_present"
        );
    }
    include!("audio_hosted_activation_tests.rs");
}
