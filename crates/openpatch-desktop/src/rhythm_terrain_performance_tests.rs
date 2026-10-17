fn rhythm_performance_patch(bypassed: bool) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"terrain","kind":"rhythm_terrain","bypassed":bypassed,"params":{"tempo":120}}],"output":"terrain","output_port":"trig1"})).unwrap()
}
#[test]
fn rhythm_terrain_callback_applies_queued_tap_release_and_publishes_real_status() {
    let patch = rhythm_performance_patch(false);
    let (mut callback, _updates, _disposal) = super::super::tests::sequence_callback(&patch);
    let performance = callback.performance.clone();
    assert!(ModuleKind::from_name("rhythm_terrain") == Some(ModuleKind::RhythmTerrain));
    assert!(ModuleKind::RhythmTerrain.manual() && ModuleKind::RhythmTerrain.has_safety_release());
    assert!(!ModuleKind::RhythmTerrain.sequential());
    assert!(performance.module_control_state("terrain").is_none());
    assert!(performance.trigger("terrain", ModuleAction::Clock).is_err());
    assert!(
        performance
            .trigger("terrain", ModuleAction::TapPress)
            .unwrap()["queued"]
            == true
    );
    callback.render(&mut [0.0_f32; 64], 1);
    let controls = performance.module_control_state("terrain").unwrap();
    assert!(controls.pressed);
    assert_eq!(controls.pressed_mask, 1);
    let lights = performance.module_lights("terrain").unwrap();
    assert!(lights.manual);
    assert_eq!(lights.count, 10);
    assert_eq!(lights.clock_frequency, Some(2.0));
    assert!(lights.waveform_position.is_some());
    assert!(
        performance
            .trigger("terrain", ModuleAction::TapRelease)
            .unwrap()["queued"]
            == true
    );
    callback.render(&mut [0.0_f32; 64], 1);
    assert_eq!(
        performance
            .module_control_state("terrain")
            .unwrap()
            .pressed_mask,
        0
    );
    assert!(!performance.module_lights("terrain").unwrap().manual);
    for action in [ModuleAction::ReleaseTap, ModuleAction::Reset] {
        assert!(performance.trigger("terrain", action).unwrap()["queued"] == true);
        callback.render(&mut [0.0_f32; 64], 1);
    }
    assert_eq!(performance.snapshot()["applied"], 4);
}
#[test]
fn rhythm_terrain_full_queue_emergency_release_cancels_queued_presses_in_actual_callback() {
    let patch = rhythm_performance_patch(false);
    let (mut callback, _updates, _disposal) = super::super::tests::sequence_callback(&patch);
    let performance = callback.performance.clone();
    performance
        .trigger("terrain", ModuleAction::TapPress)
        .unwrap();
    callback.render(&mut [0.0_f32; 64], 1);
    assert!(performance.module_control_state("terrain").unwrap().pressed);
    for _ in 0..EVENT_CAPACITY {
        performance
            .trigger("terrain", ModuleAction::TapPress)
            .unwrap();
    }
    let release = performance
        .trigger("terrain", ModuleAction::TapRelease)
        .unwrap();
    assert_eq!(release["emergency_release"], true);
    assert_eq!(release["queued"], false);
    callback.render(&mut [0.0_f32; 64], 1);
    assert!(!performance.module_control_state("terrain").unwrap().pressed);
    let snapshot = performance.snapshot();
    assert_eq!(snapshot["emergency_release_requests"], 1);
    assert_eq!(snapshot["emergency_release_applied"], 1);
    assert_eq!(
        snapshot["applied"], 1,
        "pre-release queued presses must not reassert TAP"
    );
    // Cancellation fences only old actions; a subsequent gesture is still accepted.
    performance
        .trigger("terrain", ModuleAction::TapPress)
        .unwrap();
    callback.render(&mut [0.0_f32; 64], 1);
    assert!(performance.module_control_state("terrain").unwrap().pressed);
}
#[test]
fn rhythm_terrain_bypass_adoption_releases_tap_and_accepts_only_tap_release() {
    let mut patch = rhythm_performance_patch(false);
    let (mut callback, mut updates, mut disposal) = super::super::tests::sequence_callback(&patch);
    let performance = callback.performance.clone();
    performance
        .trigger("terrain", ModuleAction::TapPress)
        .unwrap();
    callback.render(&mut [0.0_f32; 64], 1);
    assert!(performance.module_control_state("terrain").unwrap().pressed);
    patch.nodes[0].bypassed = true;
    performance.sync_patch(&patch, 2);
    for action in [
        ModuleAction::TapPress,
        ModuleAction::ReleaseTap,
        ModuleAction::Reset,
    ] {
        assert!(
            performance.trigger("terrain", action).is_err(),
            "only source TAP release works while bypassed"
        );
    }
    assert!(performance
        .trigger("terrain", ModuleAction::TapRelease)
        .is_ok());
    updates
        .push(super::super::tests::sequence_graph(&callback, &patch, 2))
        .ok()
        .unwrap();
    callback.render(&mut [0.0_f32; 256], 1);
    while disposal.pop().is_ok() {}
    assert_eq!(
        performance
            .module_control_state("terrain")
            .unwrap()
            .pressed_mask,
        0
    );
    assert!(!performance.module_lights("terrain").unwrap().manual);
    let snapshot = performance.snapshot();
    assert_eq!(snapshot["modules"]["terrain"]["bypassed"], true);
    assert_eq!(snapshot["modules"]["terrain"]["revision"], 2);
    patch.nodes[0].bypassed = false;
    performance.sync_patch(&patch, 3);
    updates
        .push(super::super::tests::sequence_graph(&callback, &patch, 3))
        .ok()
        .unwrap();
    callback.render(&mut [0.0_f32; 256], 1);
    while disposal.pop().is_ok() {}
    assert!(!performance.module_control_state("terrain").unwrap().pressed);
}
#[test]
fn rhythm_terrain_obsolete_document_action_cannot_target_reused_id_or_publish_old_packet() {
    let patch = rhythm_performance_patch(false);
    let (mut callback, mut updates, mut disposal) = super::super::tests::sequence_callback(&patch);
    let performance = callback.performance.clone();
    performance
        .trigger("terrain", ModuleAction::TapPress)
        .unwrap();
    // Remove and recreate the same textual ID before audio consumes the press.
    performance.retire_modules(&["terrain".into()]);
    performance.sync_patch(&patch, 2);
    assert!(performance.module_control_state("terrain").is_none());
    updates
        .push(super::super::tests::sequence_graph(&callback, &patch, 2))
        .ok()
        .unwrap();
    callback.render(&mut [0.0_f32; 256], 1);
    while disposal.pop().is_ok() {}
    assert!(!performance.module_control_state("terrain").unwrap().pressed);
    let snapshot = performance.snapshot();
    assert_eq!(snapshot["modules"]["terrain"]["revision"], 2);
    assert_eq!(snapshot["applied"], 0);
    assert!(snapshot["dropped"].as_u64().unwrap() >= 1);
    performance
        .trigger("terrain", ModuleAction::TapPress)
        .unwrap();
    callback.render(&mut [0.0_f32; 64], 1);
    assert!(performance.module_control_state("terrain").unwrap().pressed);
}

#[test]
fn rhythm_terrain_fast_click_ticket_preserves_host_high_low_until_source_scans_and_next_tap_locks()
{
    let patch = rhythm_performance_patch(false);
    let (mut callback, _updates, _disposal) = super::super::tests::sequence_callback(&patch);
    let performance = callback.performance.clone();
    let mut first = performance
        .press_manual_with_ticket("terrain", ModuleAction::TapPress, ModuleAction::TapRelease)
        .unwrap();
    assert!(
        !first.ready_to_release(),
        "a fast click must wait for the callback to capture HIGH"
    );
    // At 48 kHz, one host sample is shorter than a source scan. The native
    // ordered input FIFO must retain this pulse even after the host goes LOW.
    callback.render(&mut [0.0_f32; 1], 1);
    assert!(performance.module_control_state("terrain").unwrap().pressed);
    assert!(first.ready_to_release());
    first.begin_release();
    assert!(!first.is_finished());
    assert!(
        performance
            .press_manual_with_ticket("terrain", ModuleAction::TapPress, ModuleAction::TapRelease)
            .is_err(),
        "a second click must wait for captured LOW"
    );
    callback.render(&mut [0.0_f32; 1], 1);
    assert!(!performance.module_control_state("terrain").unwrap().pressed);
    assert!(first.is_finished());
    drop(first);
    // Complete a half-second interval. The first press reaches the 8 kHz
    // source scan from its FIFO, establishing the tap interval's first edge.
    callback.render(&mut [0.0_f32; 23_998], 1);
    assert_eq!(
        performance.module_lights("terrain").unwrap().brightness[8][0],
        0.0
    );
    let mut second = performance
        .press_manual_with_ticket("terrain", ModuleAction::TapPress, ModuleAction::TapRelease)
        .unwrap();
    assert!(!second.ready_to_release());
    callback.render(&mut [0.0_f32; 1], 1);
    assert!(performance.module_control_state("terrain").unwrap().pressed);
    assert!(second.ready_to_release());
    second.begin_release();
    callback.render(&mut [0.0_f32; 64], 1);
    assert!(second.is_finished());
    assert!(!performance.module_control_state("terrain").unwrap().pressed);
    let lights = performance.module_lights("terrain").unwrap();
    assert_eq!(
        lights.brightness[8][0], 1.0,
        "both short clicks reached the source tap detector"
    );
    assert_eq!(lights.clock_frequency, Some(2.0));
    performance
        .trigger("terrain", ModuleAction::ReleaseTap)
        .unwrap();
    callback.render(&mut [0.0_f32; 64], 1);
    assert_eq!(
        performance.module_lights("terrain").unwrap().brightness[8][0],
        0.0
    );
}
