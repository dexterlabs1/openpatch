// The Stock Scope bridge is exercised through the real callback and Session,
// without opening an audio device or substituting an offline scope capture.
fn stock_scope_audio_patch(mixed: bool) -> Patch {
    let mut x_params = serde_json::json!({"channels":16});
    let mut y_params = serde_json::json!({"channels":16});
    for voice in 1..=16 {
        x_params[format!("v{voice}")] = serde_json::json!(voice as f32 - 8.0);
        y_params[format!("v{voice}")] = serde_json::json!(voice as f32 * -0.125);
    }
    let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[
            {"id":"x_source","kind":"poly_constant","params":x_params},
            {"id":"y_source","kind":"poly_constant","params":y_params},
            {"id":"stock","kind":"stock_scope","params":{"time":7,"threshold":-0.25}}
        ],
        "cables":[
            {"from":"x_source","to":"stock","input":"x"},
            {"from":"y_source","to":"stock","input":"y"}
        ],
        "output":"x_source"
    })).unwrap();
    if mixed {
        let mut legacy = patch.nodes[2].clone();
        legacy.id = "legacy".into();
        legacy.kind = "scope".into();
        legacy.params.clear();
        legacy.params.insert("time".into(), 0.005);
        patch.nodes.push(legacy);
        let mut cable = patch.cables[0].clone();
        cable.to = "legacy".into();
        patch.cables.push(cable);
    }
    patch.assign_missing_cable_ids().unwrap();
    patch
}

#[test]
fn stock_scope_actual_callback_publishes_source_bins_to_session_without_history_edits() {
    let patch = stock_scope_audio_patch(false);
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone())).unwrap();
    let unavailable = session.execute("inspect_scope", serde_json::json!({"id":"stock"})).unwrap();
    assert_eq!(unavailable["scope"]["available"], false);
    let unchanged = session.snapshot();

    // Muting speaker playback must not replace the source's raw capture.
    callback.shared.playing.store(false, Ordering::Relaxed);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    let frame = callback.performance.scope_frame("stock").unwrap();
    let handle = callback.graphs.active.engine.resolve_module_scope("stock").unwrap();
    let active = callback.graphs.active.engine.module_scope(handle).unwrap();
    assert_eq!((frame.revision, frame.frame, frame.sequence), (1, 256, 1));
    assert_eq!((frame.snapshot.x_channels, frame.snapshot.y_channels), (16, 16));
    assert_eq!(frame.snapshot.x, active.x);
    assert_eq!(frame.snapshot.y, active.y);
    assert_eq!(frame.snapshot.stock, active.stock);
    assert_ne!(frame.snapshot.x[15], frame.snapshot.y[15]);
    let stock = frame.snapshot.stock.unwrap();

    let inspected = session.execute("inspect_scope", serde_json::json!({
        "id":"stock","channel":16,"expected_revision":1
    })).unwrap();
    let capture = &inspected["scope"];
    assert_eq!(capture["available"], true);
    assert_eq!(capture["model"], "stock_scope");
    assert_eq!(capture["instance"], frame.token);
    assert_eq!(capture["sequence"], frame.sequence);
    assert_eq!(capture["stock"]["raw_controls"]["time"], 7.0);
    assert_eq!(capture["stock"]["raw_controls"]["threshold"], -0.25);
    assert_eq!(capture["stock"]["parameter_bits"][4], 7.0_f32.to_bits());
    assert_eq!(capture["stock"]["light_bits"], serde_json::json!(stock.light_bits));
    assert_eq!(capture["stock"]["acquisition_sample_rate"], 48000);
    assert_eq!(capture["stock"]["next_bin_samples"], stock.next_bin_samples);
    assert_eq!(capture["stock"]["nominal_screen_seconds"], serde_json::json!(stock.nominal_screen_seconds));
    assert_eq!(capture["sample_rate"], 48000);
    assert_eq!(capture["trigger_source"], "x");
    assert_eq!(capture["trigger_input_channels"], 0);
    assert_eq!(capture["measurements"]["basis"], "completed_display_bins");
    assert_eq!(session.snapshot(), unchanged);
}

#[test]
fn stock_and_legacy_scopes_share_one_callback_publication_budget_and_keep_kind_identity() {
    let patch = stock_scope_audio_patch(true);
    let (mut callback, mut updates, _retired) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let first = callback.performance.scope_frame("stock").unwrap();
    assert!(first.snapshot.stock.is_some());
    assert!(callback.performance.scope_frame("legacy").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let legacy = callback.performance.scope_frame("legacy").unwrap();
    assert!(legacy.snapshot.stock.is_none());
    assert_eq!((first.frame, legacy.frame), (1, 2));
    assert_eq!((first.sequence, legacy.sequence), (1, 1));

    // Reusing the Stock ID for the other model must invalidate its old reader
    // before the new graph publishes, even though both models share transport.
    let mut replaced = patch;
    let stock = replaced.nodes.iter_mut().find(|node| node.id == "stock").unwrap();
    stock.kind = "scope".into();
    stock.params.clear();
    stock.params.insert("time".into(), 0.005);
    callback.performance.sync_patch(&replaced, 2);
    assert!(callback.performance.scope_frame("stock").is_none());
    assert!(updates.push(sequence_graph(&callback, &replaced, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let fresh = callback.performance.scope_frame("stock").unwrap();
    assert_ne!(fresh.token, first.token);
    assert!(fresh.snapshot.stock.is_none());
    assert_eq!(fresh.revision, 2);
}
