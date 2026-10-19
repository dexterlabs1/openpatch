// Actual CoreAudio/CPAL output qualification, intentionally opt-in and isolated.
mod real_device {
    use super::*;
    use openpatch::control::Session;
    use openpatch::project_document::{DocumentNode, ProjectDocument, ProjectLayout};
    use serde_json::{json, Value};
    use std::time::{Duration, Instant};

    fn representative_document() -> ProjectDocument {
        let (_, reference) = hosted_document();
        let module = reference
            .graph
            .nodes
            .iter()
            .find_map(|node| {
                if let DocumentNode::Hosted { module, .. } = node {
                    Some(module.clone())
                } else {
                    None
                }
            })
            .unwrap();
        let native: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[
                {"id":"pitch","kind":"poly_constant","params":{"channels":16,"v1":-1,"v2":-0.9,"v3":-0.8,"v4":-0.7,"v5":-0.6,"v6":-0.5,"v7":-0.4,"v8":-0.3,"v9":-0.2,"v10":-0.1,"v11":0,"v12":0.1,"v13":0.2,"v14":0.3,"v15":0.4,"v16":0.5}},
                {"id":"vco","kind":"stock_vco","params":{"frequency":-12}},
                {"id":"filter","kind":"stock_vcf","params":{"frequency":0.7,"resonance":0.2}},
                {"id":"amp","kind":"stock_vca","params":{"level":0.025,"response":1}}
            ],"cables":[
                {"id":"pitch-cable","from":"pitch","output":"out","to":"vco","input":"pitch"},
                {"id":"voice-cable","from":"vco","output":"sine","to":"filter","input":"signal"}
            ],"output":"amp","output_port":"out"})).unwrap();
        let mut document = ProjectDocument::from_patch(native, ProjectLayout::default()).unwrap();
        document.graph.nodes.push(DocumentNode::Hosted {
            id: "binary".into(),
            module,
            source: None,
        });
        for route in [
            json!({"id":"host-in","from":"filter","output":"lowpass","to":"binary","input":"in"}),
            json!({"id":"host-out","from":"binary","output":"out","to":"amp","input":"signal"}),
        ] {
            document
                .graph
                .cables
                .push(serde_json::from_value(json!({"route":route})).unwrap());
        }
        for (index, node) in document.graph.nodes.iter().enumerate() {
            document
                .layout
                .positions
                .insert(node.id().into(), [index as f32 * 180., 0.]);
        }
        document
    }

    #[test]
    #[ignore = "opens a real macOS audio OUTPUT device for 30-300 seconds; explicit companion/SDK/plugin and receipt path required"]
    fn actual_macos_device_sustained_hosted_deadlines() {
        let destination = std::path::PathBuf::from(
            std::env::var_os("OPENPATCH_HOSTED_DEVICE_REVIEW")
                .expect("explicit device receipt path"),
        );
        std::fs::create_dir_all(&destination).unwrap();
        let seconds = std::env::var("OPENPATCH_HOSTED_DEVICE_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60)
            .clamp(30, 300);
        let selected = std::env::var("OPENPATCH_HOSTED_DEVICE_NAME").ok();
        let workspace = tempfile::tempdir_in(&destination).unwrap();
        let mut session = Session::new(workspace.path()).unwrap();
        session
            .replace_project_document(representative_document(), None)
            .unwrap();
        session
            .execute("set_hosted_execution", json!({"enabled":true}))
            .unwrap();
        let ticket = session.audio_snapshot().unwrap();
        DEVICE_ALLOCATIONS.store(0, Ordering::Relaxed);
        DEVICE_DEALLOCATIONS.store(0, Ordering::Relaxed);
        DEVICE_CALLBACKS.store(0, Ordering::Relaxed);
        DEVICE_PLAYBACK_MIN_NS.store(u64::MAX, Ordering::Relaxed);
        DEVICE_PLAYBACK_MAX_NS.store(0, Ordering::Relaxed);
        DEVICE_TRACKING.store(true, Ordering::Release);
        struct StopTracking;
        impl Drop for StopTracking {
            fn drop(&mut self) {
                DEVICE_TRACKING.store(false, Ordering::Release);
            }
        }
        let _tracking = StopTracking;
        let start = Instant::now();
        let opened = Audio::new_with_document(ticket.clone(), selected.as_deref(), true);
        let mut failure = None;
        let mut observations = Vec::new();
        let mut device = json!({"requested":selected});
        let mut timing = Value::Null;
        let mut peak = 0f32;
        let mut frames = 0u64;
        let mut edits = 0;
        let mut stream_error = false;
        let mut runtime = Value::Null;
        let mut activated = false;
        let mut owners = Value::Null;
        match opened {
            Err(error) => failure = Some(error),
            Ok(mut audio) => {
                device = json!({"name":audio.device_name,"sample_rate":audio.sample_rate,"channels":audio.device_channels,"requested_buffer_frames":128});
                session
                    .set_live_module_events(Some(audio.performance.clone()))
                    .unwrap();
                session.acknowledge_audio_snapshot(ticket.id);
                audio.set_volume(0.08); // Quiet, genuine nonzero output to the selected device.
                audio.set_playing(true);
                let run = Instant::now();
                while run.elapsed() < Duration::from_secs(seconds) {
                    audio.service();
                    stream_error |= audio.take_stream_error();
                    let actual = audio.performance.snapshot();
                    runtime = actual["hosted_runtime"].clone();
                    timing = actual["audio_timing"].clone();
                    activated |= actual["available"] == true && runtime["status"] == "running";
                    peak = peak.max(audio.peak());
                    frames = audio.shared.frame_clock.load(Ordering::Acquire);
                    if stream_error || runtime["status"] == "suspended_fault" {
                        failure = Some(format!("device or hosted execution failed: {runtime}"));
                        break;
                    }
                    if run.elapsed().as_secs() >= (edits + 1) * 10 && edits < 2 {
                        let value = if edits == 0 { 0.65 } else { 0.7 };
                        if let Err(error) = session
                            .execute(
                                "set_parameter",
                                json!({"id":"filter","parameter":"frequency","value":value}),
                            )
                            .and_then(|_| {
                                let ticket = session.audio_snapshot()?;
                                audio.submit_document(ticket.clone())?;
                                session.acknowledge_audio_snapshot(ticket.id);
                                Ok(())
                            })
                        {
                            failure = Some(error);
                            break;
                        }
                        if let Err(error) = audio.set_scope(
                            session.patch(),
                            Some(ScopeSelection {
                                x: Some(ScopeEndpoint {
                                    id: "amp".into(),
                                    port: "out".into(),
                                    channel: 0,
                                }),
                                y: None,
                            }),
                        ) {
                            failure = Some(error);
                            break;
                        }
                        observations.push(json!({"elapsed_ms":run.elapsed().as_millis(),"native_and_scope_edit":edits+1,"timing":timing,"runtime":runtime}));
                        edits += 1;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                owners = audio.hosted_runtime_status();
                observations.push(json!({"elapsed_ms":run.elapsed().as_millis(),"timing":timing,"runtime":runtime,"device_frames":frames}));
                audio.set_playing(false);
                drop(audio); // Joins the actual stream before process cleanup and receipt reads.
            }
        }
        let allocations = DEVICE_ALLOCATIONS.load(Ordering::Acquire);
        let deallocations = DEVICE_DEALLOCATIONS.load(Ordering::Acquire);
        let callbacks = DEVICE_CALLBACKS.load(Ordering::Acquire);
        let min_ahead = DEVICE_PLAYBACK_MIN_NS.load(Ordering::Acquire);
        let max_ahead = DEVICE_PLAYBACK_MAX_NS.load(Ordering::Acquire);
        let passed = failure.is_none()
            && activated
            && peak > 0.
            && edits == 2
            && frames >= (seconds - 1) * 48_000
            && callbacks > 0
            && timing["overruns"] == 0
            && timing["invalid_callbacks"] == 0
            && runtime["status"] == "running"
            && allocations == 0
            && deallocations == 0;
        let receipt = json!({"passed":passed,"scope":"real native macOS CPAL output stream and actual installed plugin; no paced/manual callback injection",
            "requested_seconds":seconds,"elapsed_seconds":start.elapsed().as_secs_f64(),"device":device,
            "representative_load":"16-voice stock VCO -> stock VCF -> installed Bogaudio OFFSET -> stock VCA; two native edits and Scope rebindings",
            "device_frames":frames,"actual_callbacks":callbacks,"activated":activated,"maximum_observed_peak":peak,
            "callback_allocations":allocations,"callback_deallocations":deallocations,"final_timing":timing,
            "final_hosted_runtime":runtime,"owners_before_stop":owners,"stream_error":stream_error,"failure":failure,"observations":observations,
            "latency":{"graph_anchor_frames":387,"graph_anchor_source":"separate actual-binary impulse receipt; not remeasured by this sustained test",
                "transport_delay_frames":384,"cpal_predicted_playback_ahead_min_ns":(min_ahead != u64::MAX).then_some(min_ahead),
                "cpal_predicted_playback_ahead_max_ns":max_ahead,"physical_loopback_measured":false},
            "qualification_limits":"one device/profile/load; processing budget overruns are not hardware xrun counts; CPAL timestamps predict DAC delivery, no physical input/output loopback"});
        std::fs::write(
            destination.join("actual-macos-device-result.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        assert!(
            passed,
            "actual device qualification failed; receipt: {}",
            destination
                .join("actual-macos-device-result.json")
                .display()
        );
    }
}
