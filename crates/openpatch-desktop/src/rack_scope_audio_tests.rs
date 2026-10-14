// Real callback/bridge coverage without opening an audio device.
fn rack_scope_patch(two_scopes: bool) -> Patch {
    let mut params = serde_json::json!({"channels":16});
    for voice in 1..=16 {
        params[format!("v{voice}")] = serde_json::json!(voice as f32 - 8.0);
    }
    let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"source","kind":"poly_constant","params":params},
            {"id":"scope","kind":"scope","params":{"time":0.005}}],
        "cables":[{"from":"source","to":"scope","input":"x"},
            {"from":"source","to":"scope","input":"y"}],
        "output":"source"}))
    .unwrap();
    if two_scopes {
        let mut second = patch.nodes[1].clone();
        second.id = "second".into();
        patch.nodes.push(second);
    }
    patch.assign_missing_cable_ids().unwrap();
    patch
}

#[test]
fn rack_scope_callback_and_mcp_publish_real_polyphonic_bins_without_heap_activity() {
    let patch = rack_scope_patch(false);
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone()));
    let before = session
        .execute("inspect_scope", serde_json::json!({"id":"scope"}))
        .unwrap();
    assert_eq!(before["scope"]["available"], false);
    // Playback mute must not manufacture silence in a raw-voltage Scope.
    callback.shared.playing.store(false, Ordering::Relaxed);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    let published = callback.performance.scope_frame("scope").unwrap();
    assert_eq!(
        (published.frame, published.revision, published.sequence),
        (64, 1, 1)
    );
    assert_eq!(
        (published.snapshot.x_channels, published.snapshot.y_channels),
        (16, 16)
    );
    assert_eq!(published.snapshot.buffer_index, 64);
    for voice in 0..16 {
        for axis in [&published.snapshot.x, &published.snapshot.y] {
            assert_eq!(axis[voice][1].min, voice as f32 - 7.0);
            assert_eq!(axis[voice][1].max, voice as f32 - 7.0);
            assert!(!axis[voice][64].min.is_finite());
        }
    }
    let unchanged = session.snapshot();
    let inspected = session
        .execute(
            "inspect_scope",
            serde_json::json!({"id":"scope","channel":16,"expected_revision":1}),
        )
        .unwrap();
    assert_eq!(inspected["scope"]["channel"], 16);
    assert_eq!(
        inspected["scope"]["x"][1],
        serde_json::json!({"min":8.0,"max":8.0})
    );
    assert!(inspected["scope"]["x"][64].is_null());
    assert_eq!(inspected["scope"]["units"], "V");
    assert_eq!(inspected["scope"]["display"]["x_volts_per_screen"], 20.0);
    assert_eq!(inspected["scope"]["trigger_source"], "x");
    assert_eq!(inspected["scope"]["trigger_input_channels"], 0);
    assert_eq!(
        inspected["scope"]["measurements"]["basis"],
        "completed_display_bins"
    );
    assert_eq!(
        inspected["scope"]["measurements"]["selected_voice"]["x"],
        serde_json::json!({"min":8.0,"max":8.0,"peak_to_peak":0.0,"observations":63})
    );
    assert_eq!(
        inspected["scope"]["measurements"]["all_active_voices"]["x"],
        serde_json::json!({"min":-7.0,"max":8.0,"peak_to_peak":15.0,"observations":1008})
    );
    assert_eq!(session.snapshot(), unchanged);
    let cached = callback.performance.scope_frame("scope").unwrap();
    assert!(Arc::ptr_eq(&cached.snapshot, &published.snapshot));
    // Acquisition advances independently of publication; the old Arc is immutable.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1600], 1));
    assert_eq!(published.snapshot.buffer_index, 64);
    let next = callback.performance.scope_frame("scope").unwrap();
    assert_eq!((next.frame, next.sequence), (1664, 2));
    assert!(!Arc::ptr_eq(&next.snapshot, &published.snapshot));
    assert!(next.snapshot.x[15]
        .iter()
        .all(|p| p.min == 8.0 && p.max == 8.0));
    callback.performance.close();
    assert!(callback.performance.scope_frame("scope").is_none());
}

#[test]
fn rack_scope_publication_is_round_robin_bounded_and_throttled() {
    let (mut callback, _, _retired) = sequence_callback(&rack_scope_patch(true));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let first = callback.performance.scope_frame("scope").unwrap();
    assert!(callback.performance.scope_frame("second").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let second = callback.performance.scope_frame("second").unwrap();
    assert_eq!((first.frame, second.frame), (1, 2));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1598], 1));
    assert_eq!(
        callback.performance.scope_frame("scope").unwrap().sequence,
        1
    );
    assert_eq!(
        callback.performance.scope_frame("second").unwrap().sequence,
        1
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        callback.performance.scope_frame("scope").unwrap().sequence,
        2
    );
    assert_eq!(
        callback.performance.scope_frame("second").unwrap().sequence,
        1
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        callback.performance.scope_frame("second").unwrap().sequence,
        2
    );
}

#[test]
fn rack_scope_mcp_reports_actual_trigger_normaling_and_empty_axes() {
    for channels in [None, Some(0), Some(1), Some(16)] {
        let mut patch = rack_scope_patch(false);
        patch.cables.retain(|cable| cable.input == "x");
        if let Some(channels) = channels {
            patch.nodes.push(serde_json::from_value(serde_json::json!({"id":"trigger","kind":"poly_constant","params":{"channels":channels}})).unwrap());
            patch.cables.push(
                serde_json::from_value(
                    serde_json::json!({"from":"trigger","to":"scope","input":"trigger"}),
                )
                .unwrap(),
            );
        }
        patch.assign_missing_cable_ids().unwrap();
        let (mut callback, _, _retired) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session.set_live_module_events(Some(callback.performance.clone()));
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        let actual = session
            .execute(
                "inspect_scope",
                serde_json::json!({"id":"scope","channel":16}),
            )
            .unwrap();
        assert_eq!(
            actual["scope"]["trigger_input_channels"],
            channels.unwrap_or(0)
        );
        assert_eq!(
            actual["scope"]["trigger_source"],
            if channels.unwrap_or(0) > 0 {
                "external"
            } else {
                "x"
            }
        );
        assert!(actual["scope"]["measurements"]["selected_voice"]["y"].is_null());
        assert!(actual["scope"]["measurements"]["all_active_voices"]["y"].is_null());
        assert_eq!(
            actual["scope"]["measurements"]["selected_voice"]["x"]["peak_to_peak"],
            0.0
        );
        assert!(!actual["scope"]["measurements"]["selected_voice"]["x"]
            .as_object()
            .unwrap()
            .contains_key("rms"));
    }
}

#[test]
fn rack_scope_revisions_reincarnation_and_retirement_never_publish_stale_capture() {
    let patch = rack_scope_patch(false);
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session.set_live_module_events(Some(callback.performance.clone()));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    let first = callback.performance.scope_frame("scope").unwrap();
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"scope","parameter":"x_offset","value":2}),
        )
        .unwrap();
    assert!(callback.performance.scope_frame("scope").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    assert!(callback.performance.scope_frame("scope").is_none());
    let current = sequence_graph(&callback, session.patch(), session.revision());
    // Preparing a stale plan afterwards must not replace the current reader.
    let stale = sequence_graph(&callback, &patch, 1);
    assert!(updates.push(current).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(callback.performance.scope_frame("scope").is_none());
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_scope("scope")
        .unwrap();
    assert_eq!(
        callback
            .graphs
            .active
            .engine
            .module_scope(handle)
            .unwrap()
            .buffer_index,
        129
    );
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1535], 1));
    let edited = callback.performance.scope_frame("scope").unwrap();
    assert_eq!((edited.revision, edited.token), (2, first.token));
    assert_eq!(edited.snapshot.x_offset, 2.0);
    let measured = edited.snapshot.measurements().x.unwrap();
    assert_eq!(
        (measured.min, measured.max, measured.peak_to_peak),
        (-7.0, 8.0, 15.0)
    );
    assert_eq!(edited.frame, 1664);
    drop(stale);
    while retired.pop().is_ok() {}
    session
        .replace_patch(patch, Some(session.revision()))
        .unwrap();
    assert!(callback.performance.scope_frame("scope").is_none());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let fresh = callback.performance.scope_frame("scope").unwrap();
    assert_ne!(fresh.token, first.token);
    assert_eq!(fresh.revision, 3);
    assert_eq!(fresh.snapshot.buffer_index, 1);
    assert!(!fresh.snapshot.x[15][1].min.is_finite());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    while retired.pop().is_ok() {}
    // Keep the retired queue full while changing graphs: endpoint destruction
    // must stay off the callback, even when consumers stop draining it.
    for revision in 4..20 {
        callback.performance.sync_patch(session.patch(), revision);
        if let Err(rtrb::PushError::Full(pending)) =
            updates.push(sequence_graph(&callback, session.patch(), revision))
        {
            without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
            while retired.pop().is_ok() {}
            without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
            assert!(updates.push(pending).is_ok());
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
    }
    while retired.pop().is_ok() {}
}
