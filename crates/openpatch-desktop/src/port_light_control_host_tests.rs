// Included inside port_light_host: these fixtures follow initial production
// ordering without opening a native stream or an audio device.
fn audio_callback(patch: &Patch, enabled: bool) -> (Audio, Callback) {
    let (incoming, updates) = RingBuffer::new(QUEUE_SIZE);
    let (recycle, retired) = RingBuffer::new(QUEUE_SIZE);
    let shared = Arc::new(Shared::new());
    shared.set_port_lights_enabled(enabled);
    shared.playing.store(true, Ordering::SeqCst);
    let control = shared.port_light_control.load(Ordering::SeqCst);
    let mut callback = Callback::new(
        patch.compile().unwrap(),
        updates,
        recycle,
        shared.clone(),
        48000,
    );
    callback
        .graphs
        .active
        .prepare_port_lights_for_control(patch, control)
        .unwrap();
    callback.performance.sync_patch(patch, 0);
    callback.graphs.active.modules = callback
        .performance
        .plan(patch, &callback.graphs.active.engine, 0)
        .unwrap();
    let audio = Audio {
        _stream: None,
        incoming,
        retired,
        pending: None,
        shared,
        sample_rate: 48000,
        device_name: "headless test".into(),
        device_channels: 1,
        midi_ui: callback.midi.take_ui().unwrap(),
        midi_hardware: None,
        scope_binding: None,
        scope_generation: 0,
        scope_packets: callback.scope_packets.take().unwrap(),
        performance: callback.performance.clone(),
        submitted_revision: 0,
        port_light_reader: reader(&callback),
    };
    (audio, callback)
}
fn control(audio: &Audio) -> u64 {
    audio.shared.port_light_control.load(Ordering::SeqCst)
}

#[test]
fn port_light_control_startup_off_skips_first_acquisition_and_publication_for_both_backends() {
    for version in [1, 2] {
        let patch = light_patch(version, 5.0);
        let (audio, mut callback) = audio_callback(&patch, false);
        assert_eq!(control(&audio) & 1, 0);
        assert!(audio.port_lights(0).is_none());
        render(&mut callback, 64);
        assert!(callback
            .graphs
            .active
            .engine
            .port_light_observation_frame()
            .is_none());
        assert_eq!(
            callback.graphs.active.engine.port_light_processed_frames(),
            64
        );
        assert_eq!(callback.frame_clock, 64);
        assert_eq!(callback.next_port_publish_frame, 0);
        assert!(audio.port_lights(0).is_none());
    }
}

#[test]
fn port_light_control_rapid_off_on_rejects_cached_packet_and_requires_real_acquisition_at_existing_phase(
) {
    for version in [1, 2] {
        let patch = light_patch(version, 5.0);
        let (audio, mut callback) = audio_callback(&patch, true);
        render(&mut callback, 64);
        let old = audio.port_lights(0).unwrap();
        let deadline = callback.next_port_publish_frame;
        audio.set_port_lights_enabled(false);
        let off = control(&audio);
        assert!(
            audio.port_lights(0).is_none(),
            "OFF hides a still-fresh cached frame immediately"
        );
        audio.set_port_lights_enabled(false);
        assert_eq!(
            control(&audio),
            off,
            "repeated OFF does not create another epoch"
        );
        audio.set_port_lights_enabled(true);
        let on = control(&audio);
        assert!(on > off && off > old.control_epoch);
        assert!(
            audio.port_lights(0).is_none(),
            "an unseen OFF→ON must never resurrect old packet"
        );
        render(&mut callback, 0); // acknowledge transition without a sample.
        assert_eq!(callback.port_light_control, on);
        assert!(callback
            .graphs
            .active
            .engine
            .port_light_observation_frame()
            .is_none());
        assert_eq!(callback.next_port_publish_frame, deadline);
        render(&mut callback, 6); // samples 64..69; the next divider tick is 70.
        assert!(callback
            .graphs
            .active
            .engine
            .port_light_observation_frame()
            .is_none());
        assert!(audio.port_lights(0).is_none());
        render(&mut callback, 1);
        assert_eq!(
            callback.graphs.active.engine.port_light_observation_frame(),
            Some(70)
        );
        assert!(
            audio.port_lights(0).is_none(),
            "acquisition cannot bypass the global publication deadline"
        );
        render(&mut callback, 1600);
        let fresh = audio.port_lights(0).unwrap();
        assert_eq!(fresh.token, old.token);
        assert_eq!(fresh.control_epoch, on);
        assert!(fresh.sequence > old.sequence);
        assert!(fresh.last_observed_frame >= 70);
    }
}

#[test]
fn port_light_control_repeated_on_is_display_only_and_preserves_detector_and_deadline() {
    let patch = light_patch(2, 5.0);
    let (audio, mut callback) = audio_callback(&patch, true);
    render(&mut callback, 64);
    let first = audio.port_lights(0).unwrap();
    let epoch = control(&audio);
    let deadline = callback.next_port_publish_frame;
    audio.set_port_lights_enabled(true);
    audio.set_port_lights_enabled(true);
    assert_eq!(control(&audio), epoch);
    render(&mut callback, 1);
    assert_eq!(
        callback.graphs.active.engine.port_light_observation_frame(),
        Some(63)
    );
    assert_eq!(callback.next_port_publish_frame, deadline);
    assert!(Arc::ptr_eq(&first, &audio.port_lights(0).unwrap()));
}

#[test]
fn port_light_control_off_survives_backend_and_same_revision_scope_adoption() {
    for next_version in [1, 2] {
        let patch = light_patch(2, 5.0);
        let (mut audio, mut callback) = audio_callback(&patch, false);
        render(&mut callback, 64);
        let replacement = light_patch(next_version, -5.0);
        audio.submit_patch_at_revision(&replacement, 1).unwrap();
        render(&mut callback, 1);
        assert_eq!(callback.graphs.active.port_light_control, control(&audio));
        assert!(callback
            .graphs
            .active
            .engine
            .port_light_observation_frame()
            .is_none());
        assert!(callback
            .graphs
            .previous
            .as_ref()
            .unwrap()
            .engine
            .port_light_observation_frame()
            .is_none());
        render(&mut callback, 512);
        audio.service();
        let selection = ScopeSelection {
            x: Some(ScopeEndpoint {
                id: "dc".into(),
                port: "out".into(),
                channel: 0,
            }),
            y: None,
        };
        audio.set_scope(&replacement, Some(selection)).unwrap();
        assert!(audio.port_lights(1).is_none());
        render(&mut callback, 1);
        assert_eq!(callback.graphs.active.revision, 1);
        assert!(callback.graphs.active.scope.enabled);
        assert!(callback
            .graphs
            .active
            .engine
            .port_light_observation_frame()
            .is_none());
        assert!(callback
            .graphs
            .previous
            .as_ref()
            .unwrap()
            .engine
            .port_light_observation_frame()
            .is_none());
        audio.set_port_lights_enabled(true);
        render(&mut callback, 1600);
        let frame = audio.port_lights(1).unwrap();
        assert_eq!(frame.backend, next_version);
        assert_eq!(frame.control_epoch, control(&audio));
        assert_eq!(
            state(&frame, "dc", PortDirection::Output, "out").raw_rgb[0],
            0.5
        );
        audio.service();
    }
}

#[test]
fn port_light_control_queued_old_intent_and_rapid_toggle_cannot_reinherit_stale_measurements() {
    let patch = light_patch(2, 5.0);
    let (mut audio, mut callback) = audio_callback(&patch, true);
    render(&mut callback, 64);
    let old = audio.port_lights(0).unwrap();
    audio.set_scope(&patch, None).unwrap(); // Same revision, prepared under old enabled epoch.
    audio.set_port_lights_enabled(false);
    audio.set_port_lights_enabled(true);
    assert!(audio.port_lights(0).is_none());
    render(&mut callback, 0);
    assert!(callback.graphs.previous.is_some());
    assert!(callback
        .graphs
        .previous
        .as_ref()
        .unwrap()
        .engine
        .port_light_observation_frame()
        .is_none());
    assert!(
        callback
            .graphs
            .active
            .engine
            .port_light_observation_frame()
            .is_none(),
        "adoption cannot restore detector history from a preceding epoch"
    );
    render(&mut callback, 6);
    assert!(callback
        .graphs
        .active
        .engine
        .port_light_observation_frame()
        .is_none());
    render(&mut callback, 1600);
    let new = audio.port_lights(0).unwrap();
    assert_ne!(new.token, old.token);
    assert_eq!(new.control_epoch, control(&audio));
    assert_eq!(new.sequence, 1);
    audio.service();
    // Deliberately enqueue an enabled graph while steady OFF to verify callback
    // enforcement independently of the production preparation path.
    audio.set_port_lights_enabled(false);
    render(&mut callback, 64);
    let next = prepare_graph(&patch, 48000, None, 3, true).unwrap();
    assert_eq!(next.port_light_control & 1, 1);
    audio
        .incoming
        .push(next)
        .unwrap_or_else(|_| panic!("queue should be empty"));
    render(&mut callback, 1);
    assert_eq!(callback.graphs.active.port_light_control, control(&audio));
    assert!(callback
        .graphs
        .active
        .engine
        .port_light_observation_frame()
        .is_none());
    assert!(callback
        .graphs
        .previous
        .as_ref()
        .unwrap()
        .engine
        .port_light_observation_frame()
        .is_none());
}

#[test]
fn port_light_control_off_keeps_audio_gain_and_scope_samples_unchanged_including_pause_and_mute() {
    let patch = light_patch(2, 0.5);
    let (mut measured, mut a) = audio_callback(&patch, true);
    let (mut baseline, mut b) = audio_callback(&patch, true);
    let selection = ScopeSelection {
        x: Some(ScopeEndpoint {
            id: "dc".into(),
            port: "out".into(),
            channel: 0,
        }),
        y: None,
    };
    measured.set_scope(&patch, Some(selection.clone())).unwrap();
    baseline.set_scope(&patch, Some(selection)).unwrap();
    let mut actual = [0.0_f32; 2048];
    let mut expected = actual;
    for (index, count) in [64, 1, 6, 256, 1024, 2048].into_iter().enumerate() {
        if index == 1 {
            measured.set_port_lights_enabled(false);
        }
        if index == 3 {
            measured.set_port_lights_enabled(true);
        }
        if index == 4 {
            measured.set_playing(false);
            baseline.set_playing(false);
        }
        if index == 5 {
            measured.set_volume(0.0);
            baseline.set_volume(0.0);
        }
        without_heap_activity(|| a.render(&mut actual[..count], 1));
        without_heap_activity(|| b.render(&mut expected[..count], 1));
        assert_eq!(actual[..count], expected[..count]);
        assert_eq!(a.gain.value, b.gain.value);
        assert_eq!(a.frame_clock, b.frame_clock);
        let packets_a: Vec<_> = measured.scope_packets().collect();
        let packets_b: Vec<_> = baseline.scope_packets().collect();
        assert_eq!(packets_a.len(), packets_b.len());
        for (left, right) in packets_a.iter().zip(&packets_b) {
            assert_eq!(left.start_frame, right.start_frame);
            assert_eq!(left.generation, right.generation);
            assert_eq!(left.x, right.x);
            assert_eq!(left.y, right.y);
            assert_eq!(left.x_active, right.x_active);
            assert_eq!(left.y_active, right.y_active);
        }
        measured.service();
        baseline.service();
    }
}

fn port_light_off_profile(patch: &Patch) -> serde_json::Value {
    use std::{hint::black_box, time::Instant};
    let (runtime_audio, mut runtime) = audio_callback(patch, true);
    let (_startup_audio, mut startup) = audio_callback(patch, false);
    let mut a = [0.0_f32; 256];
    let mut b = a;
    for _ in 0..64 {
        runtime.render(&mut a, 1);
        startup.render(&mut b, 1);
    }
    runtime_audio.set_port_lights_enabled(false);
    for _ in 0..64 {
        runtime.render(&mut a, 1);
        startup.render(&mut b, 1);
    }
    let mut pairs = Vec::new();
    for iteration in 0..16 {
        let mut run = |is_runtime: bool| {
            let callback = if is_runtime {
                &mut runtime
            } else {
                &mut startup
            };
            let output = if is_runtime { &mut a } else { &mut b };
            let start = Instant::now();
            for _ in 0..128 {
                callback.render(black_box(output), 1);
            }
            start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
        };
        let (changed, initial) = if iteration % 2 == 0 {
            let changed = run(true);
            (changed, run(false))
        } else {
            let initial = run(false);
            (run(true), initial)
        };
        assert_eq!(a, b);
        pairs.push(
            serde_json::json!({"runtime_off_ns":changed,"startup_off_ns":initial,"callbacks":128}),
        );
    }
    assert!(runtime
        .graphs
        .active
        .engine
        .port_light_observation_frame()
        .is_none());
    assert!(startup
        .graphs
        .active
        .engine
        .port_light_observation_frame()
        .is_none());
    serde_json::json!({"profile":"port_light_off_fast_path","voices":16,"nodes":65,"block_frames":256,"pairs":pairs})
}
