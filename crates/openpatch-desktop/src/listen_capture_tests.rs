use super::*;

fn listen_handle() -> (openpatch_core::Engine, ModuleHandle) {
    let patch: openpatch_core::Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"listen","kind":"rackmcp_listen"}],
        "output":"listen","output_port":"left"}))
    .unwrap();
    let engine = patch.compile().unwrap();
    let handle = engine.resolve_module_lights("listen").unwrap();
    (engine, handle)
}

fn start(service: &CaptureService, workspace: &Path, write_wav: bool) -> String {
    service
        .start(StartRequest {
            target: Target { slot: 3, token: 17 },
            module: "listen",
            revision: 7,
            rate: 8000,
            seconds: 0.1,
            write_wav,
            workspace,
        })
        .unwrap()["job"]
        .as_str()
        .unwrap()
        .into()
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Listen worker did not reach expected state"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn arm(producer: &mut CaptureProducer, handle: ModuleHandle) {
    wait_for(|| {
        let resolved = producer.prepare(|target| {
            assert_eq!((target.slot, target.token), (3, 17));
            Some(handle)
        });
        if resolved.is_some() {
            producer.observe_status(ListenStatus {
                left_connected: true,
                right_connected: true,
                ..ListenStatus::default()
            });
        }
        resolved.is_some()
    });
    assert!(producer.armed());
}

fn finished(service: &CaptureService, id: &str) -> Value {
    let mut result = Value::Null;
    wait_for(|| {
        result = service.poll(id).unwrap();
        matches!(result["state"].as_str(), Some("done" | "failed"))
    });
    result
}

fn push_frames(producer: &mut CaptureProducer, count: usize, left: f32, right: f32) {
    assert!(producer.armed());
    for _ in 0..count {
        assert!(producer.push(ListenFrame { left, right }));
    }
}

#[test]
fn completion_reuse_acknowledges_stop_before_clearing_old_inflight_frames() {
    let workspace = tempfile::tempdir().unwrap();
    let (_engine, handle) = listen_handle();
    let (service, mut producer) = channel();
    let first = start(&service, workspace.path(), false);
    arm(&mut producer, handle);
    push_frames(&mut producer, 800, 2.0, -3.0);
    let result = finished(&service, &first);
    assert_eq!(result["state"], "done", "{result}");
    assert_eq!(result["collected_frames"], 800);
    // The callback has not observed the worker's STOP. A render already in
    // flight can still append old frames while analysis finishes.
    push_frames(&mut producer, 32, 99.0, -99.0);
    let second = start(&service, workspace.path(), false);
    wait_for(|| {
        service
            .shared
            .read()
            .is_some_and(|command| command.job == 2 && !command.armed)
    });
    push_frames(&mut producer, 32, 99.0, -99.0);
    let waiting = service.poll(&second).unwrap();
    assert_eq!(waiting["phase"], "arming");
    assert_eq!(waiting["collected_frames"], 0);
    // This is the first reuse STOP acknowledgment; only after it may the
    // worker clear every preceding frame and publish the new ARM command.
    assert!(producer.prepare(|_| Some(handle)).is_none());
    arm(&mut producer, handle);
    push_frames(&mut producer, 800, 4.0, -5.0);
    let result = finished(&service, &second);
    assert_eq!(result["state"], "done", "{result}");
    assert_eq!(result["collected_frames"], 800);
    assert_eq!(
        result["result"]["analysis"]["channels"][0]["peakVolts"],
        4.0
    );
    assert_eq!(
        result["result"]["analysis"]["channels"][1]["peakVolts"],
        5.0
    );
    assert_eq!(result["result"]["analysis"]["channels"][0]["dcVolts"], 4.0);
    assert_eq!(result["result"]["analysis"]["channels"][1]["dcVolts"], -5.0);
    service.close();
}

#[test]
fn cancellation_invalidation_and_close_fail_jobs_before_arm_without_callbacks() {
    let workspace = tempfile::tempdir().unwrap();
    let (_engine, handle) = listen_handle();
    for action in ["cancel", "invalidate", "queued_close"] {
        let (service, mut producer) = channel();
        let id = start(&service, workspace.path(), false);
        match action {
            "cancel" => {
                let receipt = service.cancel(&id).unwrap();
                assert_eq!(receipt["cancel_requested"], true);
            }
            "invalidate" => {
                wait_for(|| {
                    service
                        .shared
                        .read()
                        .is_some_and(|command| command.job == 1)
                });
                producer.invalidate();
            }
            _ => service.close(),
        }
        let result = finished(&service, &id);
        assert_eq!(result["state"], "failed", "{action}: {result}");
        assert_eq!(result["collected_frames"], 0);
        assert!(!producer.armed());
        assert!(producer.prepare(|_| Some(handle)).is_none());
        assert!(!producer.armed());
        if action == "cancel" {
            // Cancelled generations cannot be revived, while a new job may use
            // the same healthy endpoint after its own stop/arm handshake.
            let next = start(&service, workspace.path(), false);
            arm(&mut producer, handle);
            push_frames(&mut producer, 800, 0.0, 0.0);
            assert_eq!(finished(&service, &next)["state"], "done");
        }
        service.close();
        assert!(service
            .start(StartRequest {
                target: Target { slot: 3, token: 17 },
                module: "listen",
                revision: 7,
                rate: 8000,
                seconds: 0.1,
                write_wav: false,
                workspace: workspace.path(),
            })
            .is_err());
    }
}

#[test]
fn live_worker_wav_reserves_a_new_name_and_preserves_existing_capture_file() {
    let workspace = tempfile::tempdir().unwrap();
    let (_engine, handle) = listen_handle();
    let existing = workspace.path().join("listen-1.wav");
    std::fs::write(&existing, b"existing recording").unwrap();
    let (service, mut producer) = channel();
    let id = start(&service, workspace.path(), true);
    arm(&mut producer, handle);
    push_frames(&mut producer, 800, 10.0, -5.0);
    let result = finished(&service, &id);
    assert_eq!(result["state"], "done", "{result}");
    assert_eq!(result["result"]["wav_file"], "listen-1-1.wav");
    assert!(result["result"]["wav_error"].is_null());
    assert_eq!(std::fs::read(existing).unwrap(), b"existing recording");
    let bytes = std::fs::read(workspace.path().join("listen-1-1.wav")).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(bytes.len(), 58 + 800 * 8);
    assert_eq!(f32::from_le_bytes(bytes[58..62].try_into().unwrap()), 1.0);
    assert_eq!(f32::from_le_bytes(bytes[62..66].try_into().unwrap()), -0.5);
    service.close();
}

#[test]
fn unacknowledged_start_and_an_armed_but_stalled_stream_fail_after_bounded_wait() {
    let workspace = tempfile::tempdir().unwrap();
    let (_engine, handle) = listen_handle();
    for acknowledge in [false, true] {
        let (service, mut producer) = channel();
        let id = start(&service, workspace.path(), false);
        if acknowledge {
            arm(&mut producer, handle);
        }
        let result = finished(&service, &id);
        assert_eq!(result["state"], "failed");
        assert_eq!(result["collected_frames"], 0);
        let error = result["result"]["error"].as_str().unwrap();
        assert!(
            error.contains(if acknowledge {
                "no audio frames"
            } else {
                "no audio callback acknowledged"
            }),
            "{error}"
        );
        assert_eq!(service.latest_for("listen").unwrap()["job"], id);
        assert!(service.latest_for("another").is_none());
        service.close();
    }
}
