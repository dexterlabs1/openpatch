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
    }))
    .unwrap();
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
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    let unavailable = session
        .execute("inspect_scope", serde_json::json!({"id":"stock"}))
        .unwrap();
    assert_eq!(unavailable["scope"]["available"], false);
    let unchanged = session.snapshot();

    // Muting speaker playback must not replace the source's raw capture.
    callback.shared.playing.store(false, Ordering::Relaxed);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    let frame = callback.performance.scope_frame("stock").unwrap();
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_scope("stock")
        .unwrap();
    let active = callback.graphs.active.engine.module_scope(handle).unwrap();
    assert_eq!((frame.revision, frame.frame, frame.sequence), (1, 256, 1));
    assert_eq!(
        (frame.snapshot.x_channels, frame.snapshot.y_channels),
        (16, 16)
    );
    assert_eq!(frame.snapshot.x, active.x);
    assert_eq!(frame.snapshot.y, active.y);
    assert_eq!(frame.snapshot.stock, active.stock);
    assert_ne!(frame.snapshot.x[15], frame.snapshot.y[15]);
    let stock = frame.snapshot.stock.unwrap();

    let inspected = session
        .execute(
            "inspect_scope",
            serde_json::json!({
                "id":"stock","channel":16,"expected_revision":1
            }),
        )
        .unwrap();
    let capture = &inspected["scope"];
    assert_eq!(capture["available"], true);
    assert_eq!(capture["model"], "stock_scope");
    assert_eq!(capture["instance"], frame.token);
    assert_eq!(capture["sequence"], frame.sequence);
    assert_eq!(capture["stock"]["raw_controls"]["time"], 7.0);
    assert_eq!(capture["stock"]["raw_controls"]["threshold"], -0.25);
    assert_eq!(capture["stock"]["parameter_bits"][4], 7.0_f32.to_bits());
    assert_eq!(
        capture["stock"]["light_bits"],
        serde_json::json!(stock.light_bits)
    );
    assert_eq!(capture["stock"]["acquisition_sample_rate"], 48000);
    assert_eq!(capture["stock"]["next_bin_samples"], stock.next_bin_samples);
    assert_eq!(
        capture["stock"]["nominal_screen_seconds"],
        serde_json::json!(stock.nominal_screen_seconds)
    );
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
    let stock = replaced
        .nodes
        .iter_mut()
        .find(|node| node.id == "stock")
        .unwrap();
    stock.kind = "scope".into();
    stock.params.clear();
    stock.params.insert("time".into(), 0.005);
    callback.performance.sync_patch(&replaced, 2);
    assert!(callback.performance.scope_frame("stock").is_none());
    assert!(updates
        .push(sequence_graph(&callback, &replaced, 2))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let fresh = callback.performance.scope_frame("stock").unwrap();
    assert_ne!(fresh.token, first.token);
    assert!(fresh.snapshot.stock.is_none());
    assert_eq!(fresh.revision, 2);
}

fn stock_scope_callback_state(callback: &Callback) -> openpatch_core::StockScopeDiagnosticBits {
    let engine = &callback.graphs.active.engine;
    engine
        .stock_scope_diagnostic_bits(engine.resolve_module_state("stock").unwrap())
        .unwrap()
}

#[test]
fn stock_scope_callback_adopts_controls_and_routes_initializes_and_recreates_source_state() {
    let patch = stock_scope_audio_patch(false);
    let y_cable = patch.cables[1].id.clone().unwrap();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 5], 1));
    let first = callback.performance.scope_frame("stock").unwrap();
    let before = stock_scope_callback_state(&callback);
    assert!(before.buffer_index > 0 && before.frame_index > 0);
    assert!(before.point_buffer[0][0][15].min_bits != f32::INFINITY.to_bits());
    assert!(before.current_points[0][15].min_bits != f32::INFINITY.to_bits());

    session.execute("apply_edits", serde_json::json!({"edits":[
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"time","value":2}},
        {"tool":"set_parameter","arguments":{"id":"stock","parameter":"threshold","value":0.75}},
        {"tool":"disconnect","arguments":{"cable_id":y_cable}},
        {"tool":"connect","arguments":{"from":"x_source","to":"stock","input":"y"}}
    ]})).unwrap();
    assert!(callback.performance.scope_frame("stock").is_none());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    // Adoption runs before the sample loop, including an empty callback. This
    // isolates retained source history from subsequent acquisition progress.
    let frame_clock = callback.frame_clock;
    without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
    assert_eq!(callback.frame_clock, frame_clock);
    let mut expected = before;
    expected.parameter_bits[4] = 2.0_f32.to_bits();
    expected.parameter_bits[6] = 0.75_f32.to_bits();
    assert_eq!(stock_scope_callback_state(&callback), expected);

    // Advance through the inherited publication deadline and the graph fade.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1600], 1));
    assert!(retired.pop().is_ok());
    while retired.pop().is_ok() {}
    let routed = callback.performance.scope_frame("stock").unwrap();
    assert_eq!(
        (routed.token, routed.revision),
        (first.token, session.revision())
    );
    let engine = &callback.graphs.active.engine;
    let raw = engine
        .host_module_snapshot(engine.resolve_module_state("stock").unwrap())
        .unwrap();
    assert_eq!(raw.inputs[1].channels, 16);
    assert_eq!(
        (raw.inputs[1].values[0], raw.inputs[1].values[15]),
        (-7.0, 8.0)
    );
    let mut partial = stock_scope_callback_state(&callback);
    assert!(partial.buffer_index > 0 && partial.frame_index > 0);
    assert!(partial.current_points[1][15].min_bits != f32::INFINITY.to_bits());
    let latest = usize::from(partial.buffer_index - 1);
    assert_eq!(
        (
            routed.snapshot.y[15][latest].min,
            routed.snapshot.y[15][latest].max
        ),
        (8.0, 8.0)
    );
    let inspected = session
        .execute(
            "inspect_scope",
            serde_json::json!({"id":"stock","channel":16}),
        )
        .unwrap();
    assert_eq!(inspected["scope"]["available"], true);
    assert_eq!(inspected["scope"]["instance"], first.token);
    assert_eq!(inspected["scope"]["revision"], session.revision());
    assert_eq!(
        inspected["scope"]["y"][latest],
        serde_json::json!({"min":8.0,"max":8.0})
    );
    assert_eq!(inspected["scope"]["stock"]["raw_controls"]["time"], 2.0);
    assert_eq!(
        inspected["scope"]["stock"]["raw_controls"]["threshold"],
        0.75
    );

    session
        .execute("initialize_modules", serde_json::json!({"ids":["stock"]}))
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
    // Initialize restores authored defaults and clears completed Points only;
    // all nontrivial partial acquisition and source observations must survive.
    partial.parameter_bits = [0.0_f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0].map(f32::to_bits);
    for point in partial.point_buffer.iter_mut().flatten().flatten() {
        point.min_bits = f32::INFINITY.to_bits();
        point.max_bits = f32::NEG_INFINITY.to_bits();
    }
    assert_eq!(stock_scope_callback_state(&callback), partial);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1600], 1));
    assert!(retired.pop().is_ok());
    while retired.pop().is_ok() {}
    let initialized = callback.performance.scope_frame("stock").unwrap();
    assert_eq!(
        (initialized.token, initialized.revision),
        (first.token, session.revision())
    );
    let inspected = session
        .execute("inspect_scope", serde_json::json!({"id":"stock"}))
        .unwrap();
    assert_eq!(inspected["scope"]["available"], true);
    assert_eq!(inspected["scope"]["stock"]["raw_controls"]["time"], 1.0);
    assert_eq!(
        inspected["scope"]["stock"]["raw_controls"]["threshold"],
        0.0
    );

    // Even when removal and same-kind recreation reach audio in one graph,
    // the incarnation fence must exclude the previous source's acquisition.
    let params = session
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == "stock")
        .unwrap()
        .params
        .clone();
    session.execute("apply_edits", serde_json::json!({"edits":[
        {"tool":"remove_module","arguments":{"id":"stock"}},
        {"tool":"add_module","arguments":{"id":"stock","kind":"stock_scope","params":params}},
        {"tool":"connect","arguments":{"from":"x_source","to":"stock","input":"x"}},
        {"tool":"connect","arguments":{"from":"x_source","to":"stock","input":"y"}}
    ]})).unwrap();
    assert!(callback.performance.scope_frame("stock").is_none());
    let graph = sequence_graph(&callback, session.patch(), session.revision());
    let cold = graph
        .engine
        .stock_scope_diagnostic_bits(graph.engine.resolve_module_state("stock").unwrap())
        .unwrap();
    assert!(updates.push(graph).is_ok());
    let frame_clock = callback.frame_clock;
    without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
    assert_eq!(callback.frame_clock, frame_clock);
    assert_eq!(stock_scope_callback_state(&callback), cold);
    let fresh = callback.performance.scope_frame("stock").unwrap();
    assert_ne!(fresh.token, first.token);
    assert_eq!(fresh.revision, session.revision());
    assert_eq!(
        (
            fresh.snapshot.buffer_index,
            fresh.snapshot.x_channels,
            fresh.snapshot.y_channels
        ),
        (0, 0, 0)
    );
    assert!(fresh
        .snapshot
        .x
        .iter()
        .flatten()
        .all(|point| point.min == f32::INFINITY && point.max == f32::NEG_INFINITY));
    let inspected = session
        .execute("inspect_scope", serde_json::json!({"id":"stock"}))
        .unwrap();
    assert_eq!(inspected["scope"]["available"], true);
    assert_eq!(inspected["scope"]["instance"], fresh.token);
    assert_eq!(inspected["scope"]["buffer_index"], 0);
    assert_eq!(inspected["scope"]["x_channels"], 0);
    assert_eq!(inspected["scope"]["y_channels"], 0);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    assert!(retired.pop().is_ok());
    while retired.pop().is_ok() {}
}
