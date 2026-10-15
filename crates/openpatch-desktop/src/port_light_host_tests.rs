mod port_light_host {
    use super::*;
    use openpatch_core::{PortDirection, PortLightState};

    fn light_patch(version: u32, value: f32) -> Patch {
        serde_json::from_value(serde_json::json!({"version":version,"sample_rate":48000,
            "nodes":[{"id":"dc","kind":"constant","params":{"value":value}},
                {"id":"sink","kind":"gain","params":{"gain":1}}],
            "cables":[{"id":"wire","from":"dc","to":"sink","input":"signal"}],"output":"sink"}))
        .unwrap()
    }
    fn reader(callback: &Callback) -> PortLightReader {
        callback
            .graphs
            .active
            .port_light_reader
            .as_ref()
            .unwrap()
            .clone()
    }
    fn state(
        frame: &PortLightFrame,
        id: &str,
        direction: PortDirection,
        port: &str,
    ) -> PortLightState {
        frame.ports[&PortLightKey::new(id, direction, port)]
    }
    fn render(callback: &mut Callback, count: usize) {
        let mut output = [0.0_f32; 4096];
        without_heap_activity(|| callback.render(&mut output[..count], 1));
    }

    #[test]
    fn port_light_callback_first_acquisition_is_real_for_both_backends_and_heap_free() {
        for version in [1, 2] {
            let patch = light_patch(version, 5.0);
            let (mut callback, _, _) = callback_for(&patch);
            let reader = reader(&callback);
            assert!(reader.read(0).is_none());
            render(&mut callback, 0);
            assert!(reader.read(0).is_none());
            // Both backends acquire from real engine processing, not a UI preview.
            render(&mut callback, 64);
            let frame = reader.read(0).unwrap();
            assert_eq!(frame.backend, version);
            assert_eq!(frame.frame, 64);
            assert_eq!(frame.sequence, 1);
            assert_eq!(frame.ports.len(), 2);
            assert!(frame.last_observed_frame < frame.frame);
            for (id, direction, port) in [
                ("dc", PortDirection::Output, "out"),
                ("sink", PortDirection::Input, "signal"),
            ] {
                let signal = state(&frame, id, direction, port);
                assert!(signal.connected);
                assert_eq!(signal.channels, 1);
                assert!(!signal.invalid);
                assert_eq!(signal.raw_rgb[1], 0.5);
            }
            // Explicit offline fixtures use the same actual states and keys.
            let offline = offline_frame(&patch, &callback.graphs.active.engine, 6).unwrap();
            assert_eq!(offline.revision, 6);
            for (key, actual) in &frame.ports {
                assert_eq!(*actual, offline.ports[key]);
            }
        }
    }

    #[test]
    fn port_light_callback_rate_is_global_across_edits_and_large_callbacks() {
        let patch = light_patch(2, 5.0);
        let (mut callback, mut updates, _disposal) = callback_for(&patch);
        let initial_reader = reader(&callback);
        render(&mut callback, 64);
        let initial = initial_reader.read(0).unwrap();
        assert_eq!(initial.sequence, 1);
        let deadline = callback.next_port_publish_frame;
        let mut next = prepare_graph(&patch, 48000, None, 1, true).unwrap();
        next.revision = 1;
        let next_reader = next.port_light_reader.as_ref().unwrap().clone();
        assert!(
            next_reader.read(1).is_none(),
            "pending graph starts unavailable"
        );
        assert!(
            initial_reader.read(1).is_none(),
            "old registry never serves a new revision"
        );
        assert!(updates.push(next).is_ok());
        render(&mut callback, 64);
        assert!(
            next_reader.read(1).is_none(),
            "adoption must not bypass the global deadline"
        );
        assert_eq!(callback.next_port_publish_frame, deadline);
        // One callback may span several publication intervals: it still sends
        // one newest capture, without catch-up bursts or per-graph reset.
        render(&mut callback, 4096);
        let first = next_reader.read(1).unwrap();
        assert_eq!(first.sequence, 1);
        assert_ne!(first.token, initial.token);
        assert_eq!(first.frame, callback.frame_clock);
        assert!(first.frame >= deadline);
        render(&mut callback, 1);
        assert_eq!(next_reader.read(1).unwrap().sequence, 1);
        render(&mut callback, 1600);
        assert_eq!(next_reader.read(1).unwrap().sequence, 2);
    }

    #[test]
    fn port_light_callback_pause_and_master_mute_preserve_real_internal_activity() {
        let patch = light_patch(2, 5.0);
        let (mut callback, _, _) = callback_for(&patch);
        callback.shared.playing.store(false, Ordering::SeqCst);
        callback
            .shared
            .volume
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        callback.gain.value = 0.0;
        let reader = reader(&callback);
        let mut output = [1.0_f32; 256];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 256]);
        let frame = reader.read(0).unwrap();
        assert_eq!(
            state(&frame, "dc", PortDirection::Output, "out").raw_rgb[1],
            0.5
        );
        assert_eq!(
            state(&frame, "sink", PortDirection::Input, "signal").raw_rgb[1],
            0.5
        );
    }

    #[test]
    fn port_light_callback_poly_silence_and_stacked_cancellation_are_not_faked() {
        let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"nodes":[
            {"id":"a","kind":"constant","params":{"value":5}},
            {"id":"b","kind":"constant","params":{"value":-5}},
            {"id":"silent","kind":"poly_constant","params":{"channels":16}},
            {"id":"sum","kind":"gain"},{"id":"poly","kind":"gain"}],
            "cables":[{"from":"a","to":"sum","input":"signal"},
                {"from":"b","to":"sum","input":"signal"},
                {"from":"silent","to":"poly","input":"signal"}],"output":"sum"}))
        .unwrap();
        // POLY CONST intentionally has nonzero musical defaults. Silence is an
        // explicit fixture value for every active lane, never an inferred default.
        let silent = patch
            .nodes
            .iter_mut()
            .find(|node| node.id == "silent")
            .unwrap();
        for lane in 1..=16 {
            silent.params.insert(format!("v{lane}"), 0.0);
        }
        let (mut callback, _, _) = callback_for(&patch);
        render(&mut callback, 128);
        let frame = reader(&callback).read(0).unwrap();
        assert_eq!(frame.ports.len(), 5, "stacked destination is deduplicated");
        assert_eq!(
            state(&frame, "sum", PortDirection::Input, "signal").raw_rgb,
            [0.0; 3]
        );
        let silent = state(&frame, "silent", PortDirection::Output, "out");
        assert_eq!(silent.channels, 16);
        assert!(silent.connected);
        assert_eq!(
            silent.raw_rgb, [0.0; 3],
            "poly connection alone cannot emit blue"
        );
        assert_eq!(
            state(&frame, "a", PortDirection::Output, "out").raw_rgb[1],
            0.5
        );
        assert_eq!(
            state(&frame, "b", PortDirection::Output, "out").raw_rgb[0],
            0.5
        );
    }

    #[test]
    fn port_light_registry_covers_twentieth_output_without_a_16_bit_mask() {
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"nodes":[
            {"id":"buttons","kind":"pulses"},{"id":"sink","kind":"gain"}],
            "cables":[{"from":"buttons","output":"gate10","to":"sink","input":"signal"}],"output":"sink"})).unwrap();
        let (mut callback, _, _) = callback_for(&patch);
        let action = ModuleAction::PulsePress(10);
        let handle = callback
            .graphs
            .active
            .engine
            .resolve_module_action("buttons", action)
            .unwrap();
        assert!(callback
            .graphs
            .active
            .engine
            .handle_module_event(handle, action));
        render(&mut callback, 64);
        let frame = reader(&callback).read(0).unwrap();
        assert_eq!(
            state(&frame, "buttons", PortDirection::Output, "gate10").raw_rgb[1],
            1.0
        );
    }

    #[test]
    fn port_light_registry_recreation_and_backend_switch_do_not_reuse_state() {
        let patch = light_patch(2, 5.0);
        let (mut callback, mut updates, mut disposal) = callback_for(&patch);
        render(&mut callback, 64);
        let old = reader(&callback).read(0).unwrap();
        let replacement = light_patch(1, -5.0);
        let mut graph = prepare_graph(&replacement, 48000, None, 1, true).unwrap();
        graph.revision = 1;
        let replacement_reader = graph.port_light_reader.as_ref().unwrap().clone();
        assert!(replacement_reader.read(1).is_none());
        assert!(updates.push(graph).is_ok());
        render(&mut callback, 4096);
        let current = replacement_reader.read(1).unwrap();
        assert_ne!(current.token, old.token);
        assert_eq!(current.backend, 1);
        assert_eq!(current.revision, 1);
        assert_eq!(
            state(&current, "dc", PortDirection::Output, "out").raw_rgb[0],
            0.5
        );
        assert_eq!(
            state(&old, "dc", PortDirection::Output, "out").raw_rgb[1],
            0.5,
            "retained capture is immutable"
        );
        drop(disposal.pop().unwrap());
    }

    #[test]
    fn port_light_old_observer_is_disabled_after_adoption_and_retirement_backpressure_keeps_ownership(
    ) {
        let patch = light_patch(2, 5.0);
        let (mut callback, mut updates, mut disposal) = callback_for(&patch);
        render(&mut callback, 64);
        for revision in 1..=3 {
            let mut graph = prepare_graph(&patch, 48000, None, revision, true).unwrap();
            graph.revision = revision;
            assert!(updates.push(graph).is_ok());
            render(&mut callback, 1);
            let previous = callback.graphs.previous.as_ref().unwrap();
            assert!(
                previous.engine.port_light_observation_frame().is_none(),
                "retiring engine observer is disabled immediately after adoption"
            );
            render(&mut callback, 512);
        }
        assert!(
            callback.graphs.waiting_retirement.is_some(),
            "full recycle queue retains producer with its graph"
        );
        let mut next = prepare_graph(&patch, 48000, None, 4, true).unwrap();
        next.revision = 4;
        assert!(updates.push(next).is_ok());
        render(&mut callback, 1600);
        assert_eq!(
            callback.graphs.active.revision, 3,
            "blocked retirement prevents another adoption"
        );
        assert_eq!(reader(&callback).read(3).unwrap().revision, 3);
        drop(disposal.pop().unwrap());
        render(&mut callback, 1600);
        assert_eq!(callback.graphs.active.revision, 4);
        // Graph/channel destruction is deliberately outside tracked rendering.
        while let Ok(graph) = disposal.pop() {
            drop(graph);
        }
    }
    #[test]
    fn port_light_connected_zero_channel_port_is_valid_and_same_revision_rebuild_gets_new_token() {
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"nodes":[
            {"id":"zero","kind":"poly_constant","params":{"channels":0}},
            {"id":"sink","kind":"gain"}],
            "cables":[{"from":"zero","to":"sink","input":"signal"}],"output":"sink"}))
        .unwrap();
        let (mut callback, mut updates, _disposal) = callback_for(&patch);
        render(&mut callback, 64);
        let first = reader(&callback).read(0).unwrap();
        for state in first.ports.values() {
            assert!(state.connected);
            assert_eq!(state.channels, 0);
            assert!(!state.invalid);
            assert_eq!(state.raw_rgb, [0.0; 3]);
        }
        let next = prepare_graph(&patch, 48000, None, 2, true).unwrap();
        let next_reader = next.port_light_reader.as_ref().unwrap().clone();
        assert!(next_reader.read(0).is_none());
        assert!(updates.push(next).is_ok());
        render(&mut callback, 4096);
        let second = next_reader.read(0).unwrap();
        assert_ne!(second.token, first.token);
        assert_eq!(second.revision, first.revision);
        assert_eq!(second.sequence, 1);
    }

    #[test]
    fn port_light_audio_pending_revision_and_renamed_endpoint_reject_old_capture_immediately() {
        let patch = light_patch(2, 5.0);
        let (mut callback, incoming, retired) = callback_for(&patch);
        let mut audio = Audio {
            _stream: None,
            incoming,
            retired,
            pending: None,
            shared: callback.shared.clone(),
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
        render(&mut callback, 64);
        let old = audio.port_lights(0).unwrap();
        let mut changed = patch.clone();
        changed.nodes[0].id = "fresh".into();
        changed.cables[0].from = "fresh".into();
        changed.nodes[0].params.insert("value".into(), -5.0);
        audio.submit_patch_at_revision(&changed, 1).unwrap();
        assert!(audio.port_lights(0).is_none());
        assert!(audio.port_lights(1).is_none());
        render(&mut callback, 64);
        assert!(
            audio.port_lights(1).is_none(),
            "new reader cannot reuse old frame while awaiting publication"
        );
        render(&mut callback, 1600);
        let new = audio.port_lights(1).unwrap();
        assert_ne!(new.token, old.token);
        assert!(!new
            .ports
            .contains_key(&PortLightKey::new("dc", PortDirection::Output, "out")));
        assert_eq!(
            state(&new, "fresh", PortDirection::Output, "out").raw_rgb[0],
            0.5
        );
        assert_eq!(
            state(&old, "dc", PortDirection::Output, "out").raw_rgb[1],
            0.5
        );
        audio.service();
    }

    #[test]
    fn port_light_prepared_observer_preserves_decay_before_retiring_observer_is_disabled() {
        let patch = light_patch(2, 5.0);
        let (mut callback, mut updates, _retired) = callback_for(&patch);
        render(&mut callback, 64); // last acquisition is frame 63; next is 70.
        let mut changed = patch.clone();
        changed.nodes[0].params.insert("value".into(), 0.0);
        let graph = prepare_graph(&changed, 48000, None, 1, true).unwrap();
        assert!(updates.push(graph).is_ok());
        render(&mut callback, 1);
        let handle = callback
            .graphs
            .active
            .engine
            .resolve_port_light("dc", PortDirection::Output, "out")
            .unwrap();
        let inherited = callback.graphs.active.engine.port_light(handle).unwrap();
        assert_eq!(
            inherited.raw_rgb[1], 0.5,
            "first adopted frame retains old observer state before next acquisition"
        );
        assert!(callback
            .graphs
            .previous
            .as_ref()
            .unwrap()
            .engine
            .port_light_observation_frame()
            .is_none());
        render(&mut callback, 64);
        let falling = callback.graphs.active.engine.port_light(handle).unwrap();
        assert!(
            falling.raw_rgb[1] > 0.0 && falling.raw_rgb[1] < 0.5,
            "real observer acquisition decays inherited state"
        );
    }

    #[test]
    #[ignore = "manual release profiling; no audio device is opened"]
    fn port_light_callback_timing_profile() {
        use std::{hint::black_box, time::Instant};
        let mut nodes = vec![serde_json::json!({"id":"source","kind":"poly_constant",
            "params":{"channels":16,"v1":5,"v2":-5,"v16":10}})];
        let mut cables = Vec::new();
        for index in 0..64 {
            nodes.push(serde_json::json!({"id":format!("gain{index}"),"kind":"gain"}));
            cables.push(serde_json::json!({"from":if index==0 {"source".to_owned()} else {format!("gain{}",index-1)},
                "to":format!("gain{index}"),"input":"signal"}));
        }
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,"nodes":nodes,
            "cables":cables,"output":"gain63"}))
        .unwrap();
        let (mut enabled, _, _) = callback_for(&patch);
        let (mut disabled, _, _) = callback_for(&patch);
        disabled.graphs.active.engine.set_port_lights_enabled(false);
        drop(disabled.graphs.active.port_lights.take());
        let mut a = [0.0_f32; 256];
        let mut b = a;
        for _ in 0..64 {
            enabled.render(&mut a, 1);
            disabled.render(&mut b, 1);
        }
        let mut observations = Vec::new();
        for repetition in 0..16 {
            let mut run = |enabled_first: bool| {
                let callback = if enabled_first {
                    &mut enabled
                } else {
                    &mut disabled
                };
                let output = if enabled_first { &mut a } else { &mut b };
                let start = Instant::now();
                for _ in 0..128 {
                    callback.render(black_box(output), 1);
                }
                start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
            };
            let (on, off) = if repetition % 2 == 0 {
                let on = run(true);
                (on, run(false))
            } else {
                let off = run(false);
                (run(true), off)
            };
            assert_eq!(a, b, "observer cannot change device audio");
            observations
                .push(serde_json::json!({"enabled_ns":on,"disabled_ns":off,"callbacks":128}));
        }
        println!("{}", port_light_off_profile(&patch));
        println!(
            "{}",
            serde_json::json!({"profile":"port_light_callback","voices":16,"nodes":65,
            "physical_endpoints":128,"block_frames":256,"pairs":observations})
        );
    }
    include!("port_light_control_host_tests.rs");
}
