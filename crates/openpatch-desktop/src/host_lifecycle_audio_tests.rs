fn host_lifecycle_patch() -> Patch {
    serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"source","kind":"poly_constant","params":{"channels":3,"v1":2,"v2":4,"v3":6}},
            {"id":"stereo","kind":"stock_mid_side","params":{}},
            {"id":"sink","kind":"mult","params":{}}
        ],
        "cables":[
            {"id":"signal","from":"source","output":"out","to":"stereo","input":"left"},
            {"id":"listen","from":"stereo","output":"mid","to":"sink","input":"signal"}
        ],"output":"sink","output_port":"out1"
    }))
    .unwrap()
}

fn host_test_audio(patch: &Patch) -> (Audio, Callback) {
    let (mut callback, incoming, retired) = callback_for(patch);
    callback.performance.sync_patch(patch, 0);
    callback.graphs.active.modules = callback
        .performance
        .plan(patch, &callback.graphs.active, 0)
        .unwrap();
    let audio = Audio {
        _stream: None,
        incoming,
        retired,
        pending: None,
        shared: callback.shared.clone(),
        sample_rate: 48000,
        device_name: "headless lifecycle test".into(),
        device_channels: 1,
        midi_ui: callback.midi.take_ui().unwrap(),
        midi_hardware: None,
        scope_binding: None,
        scope_generation: 0,
        scope_packets: callback.scope_packets.take().unwrap(),
        performance: callback.performance.clone(),
        submitted_revision: 0,
        accepted_patch: patch.clone(),
        accepted_document: None,
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        hosted_controller: hosted_audio::HostedAudioController::default(),
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        hosted_control: None,
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        hosted_authored_revision: None,
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        hosted_native_intent: Vec::new(),
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        last_hosted_retirement: None,
        prepared_wave_banks: patch
            .prepare_wave_banks(&WavePreparationCache::default())
            .unwrap(),
        port_light_reader: callback
            .graphs
            .active
            .port_light_reader
            .as_ref()
            .unwrap()
            .clone(),
    };
    (audio, callback)
}

fn host_left_roundtrip() -> HostLifecycleDelta {
    let mut delta = HostLifecycleDelta::default();
    let module = delta.modules.entry("stereo".into()).or_default();
    module.inputs.insert(
        "left".into(),
        openpatch_core::InputLifecycleTransform::disconnect()
            .then(openpatch_core::InputLifecycleTransform::connect()),
    );
    module.outputs.insert(
        "mid".into(),
        openpatch_core::OutputLifecycleTransform::bypass_edge()
            .then(openpatch_core::OutputLifecycleTransform::bypass_edge()),
    );
    delta
}

#[test]
fn ordered_host_edges_apply_once_after_callback_adoption_without_heap_work() {
    let patch = host_lifecycle_patch();
    let (mut audio, mut callback) = host_test_audio(&patch);
    callback.render(&mut [0.0_f32; 8], 1);
    audio
        .submit_patch_with_host_lifecycle(
            &patch,
            1,
            audio.prepared_wave_banks.clone(),
            &host_left_roundtrip(),
        )
        .unwrap();
    without_heap_activity(|| callback.render(&mut [] as &mut [f32], 1));
    let engine = &callback.graphs.active.engine;
    let state = engine
        .host_module_snapshot(engine.resolve_module_state("stereo").unwrap())
        .unwrap();
    assert_eq!(state.inputs[1].channels, 1);
    assert!(state.inputs[1].values.iter().all(|v| v.to_bits() == 0));
    assert_eq!(state.outputs[0].channels, 1);
    assert!(state.outputs[0].values.iter().all(|v| v.to_bits() == 0));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 4], 1));
    let before = callback
        .graphs
        .active
        .engine
        .host_module_snapshot(
            callback
                .graphs
                .active
                .engine
                .resolve_module_state("stereo")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(before.outputs[0].channels, 3);
    assert_eq!(before.outputs[0].values[0], 1.0);
    without_heap_activity(|| callback.render(&mut [] as &mut [f32], 1));
    let after = callback
        .graphs
        .active
        .engine
        .host_module_snapshot(
            callback
                .graphs
                .active
                .engine
                .resolve_module_state("stereo")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn queue_full_coalescing_and_scope_replacement_keep_ordered_host_effects() {
    let patch = host_lifecycle_patch();
    let (mut audio, mut callback) = host_test_audio(&patch);
    callback.render(&mut [0.0_f32; 8], 1);
    // Fill the real bounded incoming queue before submitting the edge segment.
    for revision in 1..=2 {
        audio
            .submit_patch_with_host_lifecycle(
                &patch,
                revision,
                audio.prepared_wave_banks.clone(),
                &HostLifecycleDelta::default(),
            )
            .unwrap();
    }
    audio
        .submit_patch_with_host_lifecycle(
            &patch,
            3,
            audio.prepared_wave_banks.clone(),
            &host_left_roundtrip(),
        )
        .unwrap();
    assert!(audio.pending.is_some());
    audio
        .submit_patch_with_host_lifecycle(
            &patch,
            4,
            audio.prepared_wave_banks.clone(),
            &HostLifecycleDelta::default(),
        )
        .unwrap();
    audio.set_scope(&patch, None).unwrap();
    let pending = audio.pending.as_ref().unwrap();
    assert_eq!(pending.revision, 4);
    assert_eq!(pending.host_lifecycle.named, host_left_roundtrip());
    // Existing FIFO graphs activate separately; the retained segment is applied
    // only when its own graph finally reaches the callback.
    for _ in 0..8 {
        callback.render(&mut [0.0_f32; 512], 1);
        audio.service();
        if callback.graphs.active.revision == 4 {
            break;
        }
    }
    assert_eq!(callback.graphs.active.revision, 4);
    assert!(callback.graphs.active.host_lifecycle.applied);
    assert!(audio.pending.is_none());
}

#[test]
fn scope_rebuild_uses_sealed_accepted_graph_not_newer_session_patch() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(directory.path()).unwrap();
    session.replace_patch(host_lifecycle_patch(), None).unwrap();
    session.discard_audio_lifecycle_through_current();
    let initial = session.patch().clone();
    let (mut audio, mut callback) = host_test_audio(&initial);
    callback.render(&mut [0.0_f32; 8], 1);
    // Leave the real incoming queue full so both the ticket and the Scope
    // rebuild use the unqueued pending graph replacement path.
    for _ in 0..2 {
        audio
            .submit_patch_with_host_lifecycle(
                &initial,
                session.revision(),
                audio.prepared_wave_banks.clone(),
                &HostLifecycleDelta::default(),
            )
            .unwrap();
    }
    session
        .execute("disconnect", serde_json::json!({"cable_id":"signal"}))
        .unwrap();
    let sealed = session.audio_snapshot().unwrap();
    session
        .execute(
            "connect",
            serde_json::json!({
                "cable_id":"signal", "from":"source", "output":"out",
                "to":"stereo", "input":"left"
            }),
        )
        .unwrap();
    session
        .execute(
            "add_module",
            serde_json::json!({"id":"future","kind":"constant"}),
        )
        .unwrap();
    let current_ui_patch = session.patch().clone();
    assert!(session.revision() > sealed.revision);
    assert!(Arc::ptr_eq(&sealed, &session.audio_snapshot().unwrap()));
    audio.submit_document(sealed.clone()).unwrap();
    assert!(session.acknowledge_audio_snapshot(sealed.id));
    assert!(audio.pending.is_some());
    let selection = ScopeSelection {
        x: Some(ScopeEndpoint {
            id: "stereo".into(),
            port: "mid".into(),
            channel: 0,
        }),
        y: None,
    };
    audio.set_scope(&current_ui_patch, Some(selection)).unwrap();
    assert!(Arc::ptr_eq(
        audio.accepted_document.as_ref().unwrap(),
        &sealed
    ));
    let pending = audio.pending.as_ref().unwrap();
    assert_eq!(pending.revision, sealed.revision);
    assert_eq!(pending.host_lifecycle.named, sealed.delta);
    assert!(
        pending.engine.resolve_module_state("future").is_err(),
        "Scope must not publish a newer UI node under an older ticket revision"
    );
    let handle = pending.engine.resolve_module_state("stereo").unwrap();
    assert_eq!(
        pending.engine.host_port_layout(handle).unwrap().2,
        0,
        "The sealed graph has no input cable; the newer UI reconnect must wait for its own ticket"
    );
    let generation = audio.scope_generation();
    let future_selection = ScopeSelection {
        x: Some(ScopeEndpoint {
            id: "future".into(),
            port: "out".into(),
            channel: 0,
        }),
        y: None,
    };
    assert!(audio
        .set_scope(&current_ui_patch, Some(future_selection.clone()))
        .is_err());
    assert_eq!(audio.scope_generation(), generation);
    assert_eq!(
        audio.pending.as_ref().unwrap().host_lifecycle.named,
        sealed.delta
    );
    for _ in 0..8 {
        callback.render(&mut [0.0_f32; 512], 1);
        audio.service();
        if callback.graphs.active.revision == sealed.revision {
            break;
        }
    }
    assert_eq!(callback.graphs.active.revision, sealed.revision);
    assert!(callback.graphs.active.host_lifecycle.applied);
    let engine = &callback.graphs.active.engine;
    let observed = engine
        .host_module_snapshot(engine.resolve_module_state("stereo").unwrap())
        .unwrap();
    assert_eq!(observed.inputs[1].channels, 0);
    assert!(observed.inputs[1]
        .values
        .iter()
        .all(|value| value.to_bits() == 0));
    let latest = session.audio_snapshot().unwrap();
    assert_eq!(latest.revision, session.revision());
    audio.submit_document(latest.clone()).unwrap();
    assert!(session.acknowledge_audio_snapshot(latest.id));
    // The existing Mid tap automatically accompanies the new ticket; the new
    // endpoint also becomes bindable only after that ticket is accepted.
    audio
        .set_scope(&current_ui_patch, Some(future_selection))
        .unwrap();
    for _ in 0..8 {
        callback.render(&mut [0.0_f32; 512], 1);
        audio.service();
        if callback.graphs.active.revision == latest.revision {
            break;
        }
    }
    assert_eq!(callback.graphs.active.revision, latest.revision);
    let engine = &callback.graphs.active.engine;
    assert!(engine.resolve_module_state("future").is_ok());
    let observed = engine
        .host_module_snapshot(engine.resolve_module_state("stereo").unwrap())
        .unwrap();
    assert_eq!(observed.inputs[1].channels, 3);
    assert_eq!(observed.inputs[1].values[..3], [2.0, 4.0, 6.0]);
}
