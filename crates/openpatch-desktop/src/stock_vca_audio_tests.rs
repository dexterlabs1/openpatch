// Real Callback::render tests, included inside audio::tests; no audio device opens.
fn stock_vca_callback_patch(voices: usize, silent: bool) -> Patch {
    let mut cv = serde_json::json!({"channels": voices});
    for lane in 0..16 {
        cv[format!("v{}", lane + 1)] = serde_json::json!((lane + 1) as f32 * 0.5);
    }
    let source = if silent {
        serde_json::json!({"id":"source","kind":"constant","params":{"value":0}})
    } else {
        serde_json::json!({"id":"source","kind":"multi_vco","params":{"frequency":173.0}})
    };
    let mut patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"cv","kind":"poly_constant","params":cv}, source,
                 {"id":"vca","kind":"stock_vca","params":{"level":0.8,"response":1}},
                 {"id":"sum","kind":"poly_sum","params":{"level":0.02}}],
        "cables":[{"from":"cv","to":"vca","input":"cv"},
                  {"from":"source","output":if silent {"out"} else {"saw"},"to":"vca","input":"signal"},
                  {"from":"vca","to":"sum","input":"signal"}],
        "output":"sum"
    })).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn stock_vca_assert_callback_gains(
    lights: openpatch_core::ModuleLightState,
    voices: usize,
    level: f32,
    response: f32,
) {
    assert_eq!(lights.count, 16);
    assert_eq!(lights.input_channels, Some(voices as u8));
    assert!(!lights.manual);
    for lane in 0..16 {
        let cv = (lane + 1) as f32 * 0.5 / 10.0;
        let expected = if lane >= voices {
            0.0
        } else if response == 0.0 {
            level * (cv * cv) * (cv * cv)
        } else {
            level * cv
        };
        assert!(
            (lights.brightness[lane][0] - expected).abs() < 1e-7,
            "lane {lane}: measured {}, expected {expected} at level {level}",
            lights.brightness[lane][0]
        );
        assert_eq!(lights.brightness[lane][1], 0.0);
    }
}

#[test]
fn stock_vca_callback_first_buffer_and_graph_crossfade_have_no_heap_activity() {
    for voices in [1, 16] {
        let mut patch = stock_vca_callback_patch(voices, false);
        let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
        assert!(callback.performance.module_lights("vca").is_none());
        let mut samples = [0.0_f32; 512];
        without_heap_activity(|| callback.render(&mut samples, 2));
        assert!(samples.iter().all(|v| v.is_finite()));
        assert!(samples.iter().any(|v| v.abs() > 1e-5));
        stock_vca_assert_callback_gains(
            callback.performance.module_lights("vca").unwrap(),
            voices,
            0.8,
            1.0,
        );
        for (index, (level, response)) in
            [(0.3, 0.0), (1.0, 1.0), (0.7, 0.0)].into_iter().enumerate()
        {
            patch.nodes[2].params.insert("level".into(), level);
            patch.nodes[2].params.insert("response".into(), response);
            let revision = index as u64 + 2;
            // Simulate an intentional level edit. Ordinary graph refreshes
            // preserve the heard value through live-parameter adoption.
            callback.performance.sync_patch_with_parameters(
                &patch,
                revision,
                &std::collections::BTreeMap::from([(
                    "vca".into(),
                    std::collections::BTreeMap::from([("level".into(), level)]),
                )]),
            );
            assert!(callback.performance.module_lights("vca").is_none());
            assert!(updates
                .push(sequence_graph(&callback, &patch, revision))
                .is_ok());
            without_heap_activity(|| callback.render(&mut samples, 2));
            let timing = callback.performance.diagnostics.snapshot();
            assert!(timing.adopted_graph && timing.crossfade);
            assert!(samples.iter().all(|v| v.is_finite()));
            stock_vca_assert_callback_gains(
                callback.performance.module_lights("vca").unwrap(),
                voices,
                level,
                response,
            );
            assert_eq!(
                callback.performance.snapshot()["modules"]["vca"]["revision"],
                revision
            );
            while retired.pop().is_ok() {}
        }
    }
}

#[test]
fn stock_vca_callback_partition_preserves_audio_and_true_gain_publication_with_silence() {
    for voices in [1, 16] {
        for silent in [false, true] {
            for response in [0.0, 1.0] {
                let mut patch = stock_vca_callback_patch(voices, silent);
                patch.nodes[2].params.insert("response".into(), response);
                let (mut full, _, _) = sequence_callback(&patch);
                let (mut split, _, _) = sequence_callback(&patch);
                let mut whole = [0.0_f32; 2050];
                let mut partitioned = whole;
                without_heap_activity(|| full.render(&mut whole, 2));
                let mut cursor = 0;
                for frames in [0, 1, 3, 64, 127, 128, 129, 573] {
                    let end = cursor + frames * 2;
                    without_heap_activity(|| split.render(&mut partitioned[cursor..end], 2));
                    cursor = end;
                }
                assert_eq!(cursor, whole.len());
                assert_eq!(whole, partitioned);
                if silent {
                    assert_eq!(whole, [0.0; 2050]);
                } else {
                    assert!(whole.iter().any(|v| v.abs() > 1e-10));
                }
                let gains = full.performance.module_lights("vca").unwrap();
                assert_eq!(split.performance.module_lights("vca"), Some(gains));
                stock_vca_assert_callback_gains(gains, voices, 0.8, response);
                // Silent audio still exposes the effective control gain.
                assert!(gains.brightness[voices - 1][0] > 0.0);
                for callback in [&full, &split] {
                    let publication = callback.performance.snapshot();
                    let module = &publication["modules"]["vca"];
                    assert_eq!(module["revision"], 1);
                    assert_eq!(module["lights"]["input_channels"], voices);
                    assert_eq!(module["lights"]["count"], 16);
                    assert!(module["state"].is_null());
                    assert!(module["controls"].is_null());
                }
            }
        }
    }
}

fn stock_vca_callback_scoped_graph(
    callback: &Callback,
    patch: &Patch,
    revision: u64,
) -> PreparedGraph {
    let binding = ScopeBinding::new(
        patch,
        ScopeSelection {
            x: Some(ScopeEndpoint {
                id: "vca".into(),
                port: "out".into(),
                channel: 0,
            }),
            y: Some(ScopeEndpoint {
                id: "vca".into(),
                port: "out".into(),
                channel: 15,
            }),
        },
    )
    .unwrap();
    let mut graph = prepare_graph(patch, 48000, Some(&binding), revision, true).unwrap();
    graph.modules = callback
        .performance
        .plan(patch, &graph.engine, revision)
        .unwrap();
    graph.revision = revision;
    graph
}

#[test]
fn stock_vca_callback_bypass_passes_raw_mono_width_and_freezes_polyphonic_gain() {
    let mut patch = stock_vca_callback_patch(16, true);
    patch.nodes[1].params.insert("value".into(), 4.0);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    callback.graphs.active = stock_vca_callback_scoped_graph(&callback, &patch, 1);
    let mut packets = callback.scope_packets.take().unwrap();
    let mut samples = [0.0_f32; 512];
    without_heap_activity(|| callback.render(&mut samples, 1));
    let frozen = callback.performance.module_lights("vca").unwrap();
    stock_vca_assert_callback_gains(frozen, 16, 0.8, 1.0);
    let mut last = None;
    while let Ok(packet) = packets.pop() {
        last = Some(packet);
    }
    let normal = last.unwrap();
    assert_eq!(normal.x_active, [true; SCOPE_PACKET_SIZE]);
    assert_eq!(normal.y_active, [true; SCOPE_PACKET_SIZE]);
    for (x, y) in normal.x.iter().zip(normal.y) {
        assert!((*x - 4.0 * frozen.brightness[0][0]).abs() < 1e-6);
        assert!((y - 4.0 * frozen.brightness[15][0]).abs() < 1e-6);
    }

    // Change both control values and width while bypassed: live gain must
    // remain frozen while the output reflects the raw mono signal cable.
    patch.nodes[0].params.insert("channels".into(), 3.0);
    patch.nodes[0].params.insert("v1".into(), 10.0);
    patch.nodes[1].params.insert("value".into(), 6.0);
    patch.nodes[2].params.insert("level".into(), 0.0);
    patch.nodes[2].params.insert("response".into(), 0.0);
    patch.nodes[2].bypassed = true;
    callback.performance.sync_patch(&patch, 2);
    assert!(updates
        .push(stock_vca_callback_scoped_graph(&callback, &patch, 2))
        .is_ok());
    without_heap_activity(|| callback.render(&mut samples, 1));
    assert!(callback.performance.diagnostics.snapshot().crossfade);
    assert_eq!(callback.performance.module_lights("vca"), Some(frozen));
    let mut last = None;
    while let Ok(packet) = packets.pop() {
        assert_eq!(packet.generation, 2);
        last = Some(packet);
    }
    let bypassed = last.unwrap();
    assert_eq!(bypassed.x, [6.0; SCOPE_PACKET_SIZE]);
    assert_eq!(bypassed.x_active, [true; SCOPE_PACKET_SIZE]);
    assert_eq!(bypassed.y, [0.0; SCOPE_PACKET_SIZE]);
    assert_eq!(bypassed.y_active, [false; SCOPE_PACKET_SIZE]);
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["vca"]["bypassed"], true);
    assert_eq!(published["modules"]["vca"]["lights"]["input_channels"], 16);
    while retired.pop().is_ok() {}
    without_heap_activity(|| callback.render(&mut samples, 1));
    assert_eq!(callback.performance.module_lights("vca"), Some(frozen));
}
