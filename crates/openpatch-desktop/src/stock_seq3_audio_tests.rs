use eframe::egui;
fn stock_seq3_transport_patch() -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"seq","kind":"stock_seq3"}],"output":"seq","output_port":"cv1"})).unwrap()
}
#[test]
fn stock_seq3_actual_callback_copublishes_state_controls_and_lights_without_heap_work() {
    let patch = stock_seq3_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch.clone(), Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    assert!(callback.performance.sequence_state("seq").is_none());
    assert!(callback.performance.module_lights("seq").is_none());
    assert!(callback.performance.module_control_state("seq").is_none());
    assert!(callback
        .performance
        .trigger("seq", ModuleAction::BPress)
        .is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    let before = callback.performance.snapshot();
    let seq = &before["modules"]["seq"];
    assert_eq!(seq["state"]["gates"], serde_json::json!(vec![true; 8]));
    assert_eq!(seq["state"]["running"], true);
    assert_eq!(seq["state"]["passthrough"], false);
    assert_eq!(seq["lights"]["count"], 19);
    assert_eq!(seq["controls"]["pressed_mask"], 0);
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_telemetry("seq")
        .unwrap();
    assert_eq!(
        callback.performance.module_lights("seq"),
        callback.graphs.active.engine.module_lights(handle)
    );
    assert_eq!(
        callback.performance.module_control_state("seq"),
        callback.graphs.active.engine.module_control_state(handle)
    );
    // All eight physical gate buttons publish held levels and durable toggles.
    for gate in 1..=8 {
        callback
            .performance
            .trigger("seq", ModuleAction::SeqGatePress(gate))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let observed = callback.performance.snapshot();
        let seq = &observed["modules"]["seq"];
        assert_eq!(seq["state"]["gates"][usize::from(gate - 1)], false);
        assert_eq!(seq["controls"]["pressed_mask"], 1_u16 << (gate + 2));
        assert_eq!(seq["frame"], observed["frame"]);
        assert_eq!(seq["revision"], session.revision());
        callback
            .performance
            .trigger("seq", ModuleAction::SeqGateRelease(gate))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback
                .performance
                .module_control_state("seq")
                .unwrap()
                .pressed_mask,
            0
        );
    }
    callback
        .performance
        .trigger("seq", ModuleAction::RunPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    callback
        .performance
        .trigger("seq", ModuleAction::RunRelease)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        callback.performance.snapshot()["modules"]["seq"]["state"]["running"],
        false
    );
    // An ordinary numerical edit adopts durable actual flags, not stale params.
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"seq","parameter":"row2_3","value":4}),
        )
        .unwrap();
    assert!(callback.performance.module_lights("seq").is_none());
    assert!(callback.performance.sequence_state("seq").is_none());
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let observed = callback.performance.snapshot();
    let seq = &observed["modules"]["seq"];
    assert_eq!(seq["state"]["running"], false);
    assert_eq!(seq["state"]["gates"], serde_json::json!(vec![false; 8]));
    assert_eq!(seq["lights"]["count"], 19);
    assert_eq!(seq["controls"]["pressed_mask"], 0);
    // Explicit full state loads are authoritative after adoption and remain coherent.
    callback
        .performance
        .load_module_saved_states(&std::collections::BTreeMap::from([(
            "seq".into(),
            openpatch_core::SavedModuleState::StockSeq3 {
                running: true,
                gates: 0x55,
                passthrough: true,
            },
        )]));
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    while disposal.pop().is_ok() {}
    let state = &callback.performance.snapshot()["modules"]["seq"]["state"];
    assert_eq!(state["running"], true);
    assert_eq!(state["passthrough"], true);
    assert_eq!(
        state["gates"],
        serde_json::json!([true, false, true, false, true, false, true, false])
    );
    callback.performance.close();
    assert!(callback.performance.sequence_state("seq").is_none());
    assert!(callback.performance.module_lights("seq").is_none());
    assert!(callback.performance.module_control_state("seq").is_none());
    assert!(callback
        .performance
        .trigger("seq", ModuleAction::ClockPress)
        .is_err());
    let (mut replacement, _, _) = sequence_callback(&stock_seq3_transport_patch());
    assert!(replacement.performance.module_lights("seq").is_none());
    without_heap_activity(|| replacement.render(&mut [0.0_f32; 64], 1));
    assert_eq!(
        replacement.performance.snapshot()["modules"]["seq"]["state"]["gates"],
        serde_json::json!(vec![true; 8])
    );
    assert!(callback.performance.module_lights("seq").is_none());
}
#[test]
fn stock_seq3_callback_safety_release_and_reused_id_never_keep_a_held_button() {
    let patch = stock_seq3_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    callback
        .performance
        .trigger("seq", ModuleAction::ClockPress)
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        callback
            .performance
            .module_control_state("seq")
            .unwrap()
            .pressed_mask,
        1
    );
    callback
        .performance
        .release_module_controls(&["seq".into()]);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        callback
            .performance
            .module_control_state("seq")
            .unwrap()
            .pressed_mask,
        0
    );
    let mut bypass = patch.clone();
    bypass.nodes[0].bypassed = true;
    callback.performance.sync_patch(&bypass, 2);
    assert!(callback
        .performance
        .trigger("seq", ModuleAction::RunPress)
        .is_err());
    assert!(callback
        .performance
        .trigger("seq", ModuleAction::ClockRelease)
        .is_ok());
    assert!(updates.push(sequence_graph(&callback, &bypass, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(
        callback
            .performance
            .module_control_state("seq")
            .unwrap()
            .pressed_mask,
        0
    );
    let removed = Patch::new_v2();
    callback.performance.sync_patch(&removed, 3);
    assert!(callback.performance.module_lights("seq").is_none());
    callback.performance.sync_patch(&patch, 4);
    assert!(callback.performance.module_lights("seq").is_none());
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    assert_eq!(
        callback
            .performance
            .module_control_state("seq")
            .unwrap()
            .pressed_mask,
        0
    );
    assert_eq!(
        callback.performance.snapshot()["modules"]["seq"]["state"]["gates"],
        serde_json::json!(vec![true; 8])
    );
}
// The transport accepts queued state intents across graph preparations. Session
// state-dependent commands separately require the latest coherent revision.
fn stock_seq3_transport_assignment(
    callback: &Callback,
    patch: &mut Patch,
    revision: u64,
    name: &str,
    value: f32,
    assignment: openpatch_core::SavedModuleState,
) {
    patch.nodes[0].params.insert(name.into(), value);
    callback.performance.sync_patch(patch, revision);
    callback
        .performance
        .load_module_saved_states(&std::collections::BTreeMap::from([(
            "seq".into(),
            assignment,
        )]));
}
#[test]
fn stock_seq3_pending_transport_assignments_merge_until_consumed_and_preserve_physical_edges() {
    use openpatch_core::SavedModuleState::StockSeq3Fields;
    let mut patch = stock_seq3_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    stock_seq3_transport_assignment(
        &callback,
        &mut patch,
        2,
        "gate1",
        0.0,
        StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 1,
            passthrough: None,
        },
    );
    stock_seq3_transport_assignment(
        &callback,
        &mut patch,
        3,
        "passthrough",
        1.0,
        StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 0,
            passthrough: Some(true),
        },
    );
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let state = &callback.performance.snapshot()["modules"]["seq"]["state"];
    assert_eq!(
        state["gates"],
        serde_json::json!([false, true, true, true, true, true, true, true])
    );
    assert_eq!(state["passthrough"], true);
    assert_eq!(state["running"], true);
    callback
        .performance
        .trigger("seq", ModuleAction::SeqGatePress(1))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    callback
        .performance
        .trigger("seq", ModuleAction::SeqGateRelease(1))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert_eq!(
        callback.performance.snapshot()["modules"]["seq"]["state"]["gates"][0],
        true
    );
    // A new transport generation after acknowledgment cannot replay gate1=false.
    stock_seq3_transport_assignment(
        &callback,
        &mut patch,
        4,
        "gate2",
        0.0,
        StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 2,
            passthrough: None,
        },
    );
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let state = &callback.performance.snapshot()["modules"]["seq"]["state"];
    assert_eq!(
        state["gates"],
        serde_json::json!([true, false, true, true, true, true, true, true])
    );
    assert_eq!(state["passthrough"], true);
    callback
        .performance
        .trigger("seq", ModuleAction::RunPress)
        .unwrap();
    callback
        .performance
        .trigger("seq", ModuleAction::SeqGatePress(8))
        .unwrap();
    stock_seq3_transport_assignment(
        &callback,
        &mut patch,
        5,
        "gate3",
        0.0,
        StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 4,
            passthrough: None,
        },
    );
    assert!(updates.push(sequence_graph(&callback, &patch, 5)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let state = &callback.performance.snapshot()["modules"]["seq"]["state"];
    assert_eq!(state["running"], false);
    assert_eq!(
        state["gates"],
        serde_json::json!([true, false, false, true, true, true, true, false])
    );
    assert_eq!(state["passthrough"], true);
    assert_eq!(
        callback
            .performance
            .module_control_state("seq")
            .unwrap()
            .pressed_mask,
        2 | (1 << 10)
    );
    callback
        .performance
        .trigger("seq", ModuleAction::RunRelease)
        .unwrap();
    callback
        .performance
        .trigger("seq", ModuleAction::SeqGateRelease(8))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
}
#[test]
fn stock_seq3_session_rejects_second_state_dependent_commit_until_current_callback_revision() {
    let patch = stock_seq3_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"seq","parameter":"gate1","value":0}),
        )
        .unwrap();
    let before = session.snapshot();
    let revision = session.revision();
    let error = session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"seq","parameter":"passthrough","value":1}),
        )
        .unwrap_err();
    assert!(error.contains("live state unavailable"));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.snapshot(), before);
    assert!(updates
        .push(sequence_graph(&callback, session.patch(), revision))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"seq","parameter":"passthrough","value":1}),
        )
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    let state = &callback.performance.snapshot()["modules"]["seq"]["state"];
    assert_eq!(state["gates"][0], false);
    assert_eq!(state["passthrough"], true);
}
#[test]
fn stock_seq3_prepared_merged_masks_do_not_replay_fields_consumed_by_an_intermediate_transport_generation(
) {
    use openpatch_core::SavedModuleState::StockSeq3Fields;
    let mut patch = stock_seq3_transport_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
    stock_seq3_transport_assignment(
        &callback,
        &mut patch,
        2,
        "gate1",
        0.0,
        StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 1,
            passthrough: None,
        },
    );
    let first = sequence_graph(&callback, &patch, 2);
    stock_seq3_transport_assignment(
        &callback,
        &mut patch,
        3,
        "passthrough",
        1.0,
        StockSeq3Fields {
            running: None,
            gates: 0,
            gate_mask: 0,
            passthrough: Some(true),
        },
    );
    let second = sequence_graph(&callback, &patch, 3);
    assert!(updates.push(first).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    assert!(
        callback.performance.module_control_state("seq").is_none(),
        "A stale intermediate graph cannot serve the latest revision"
    );
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_telemetry("seq")
        .unwrap();
    assert!(callback
        .graphs
        .active
        .engine
        .handle_module_event(handle, ModuleAction::SeqGatePress(1)));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(callback
        .graphs
        .active
        .engine
        .handle_module_event(handle, ModuleAction::SeqGateRelease(1)));
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    assert!(updates.push(second).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
    while disposal.pop().is_ok() {}
    let state = &callback.performance.snapshot()["modules"]["seq"]["state"];
    assert_eq!(
        state["gates"][0], true,
        "An already consumed gate1 assignment cannot replay from a prepared merged graph"
    );
    assert_eq!(state["passthrough"], true);
}
#[test]
fn stock_seq3_fast_click_tickets_wait_for_source_sampling_for_all_eleven_buttons() {
    let pairs = [
        (ModuleAction::ClockPress, ModuleAction::ClockRelease),
        (ModuleAction::RunPress, ModuleAction::RunRelease),
        (ModuleAction::ResetPress, ModuleAction::ResetRelease),
    ];
    for index in 0..11 {
        let patch = stock_seq3_transport_patch();
        let (mut callback, _, _) = sequence_callback(&patch);
        callback.shared.playing.store(false, Ordering::SeqCst);
        assert!(!callback.shared.playing.load(Ordering::SeqCst));
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        // Reset needs a later selected step to expose an actual transition.
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let before = callback.performance.sequence_state("seq").unwrap();
        let expected_pair = if index < 3 {
            pairs[index]
        } else {
            (
                ModuleAction::SeqGatePress((index - 2) as u8),
                ModuleAction::SeqGateRelease((index - 2) as u8),
            )
        };
        // The real shared rack renderer emits its same-frame click action.
        let ctx = egui::Context::default();
        let mut rack = crate::rack::Rack::default();
        rack.set_sequence_states(std::collections::BTreeMap::new(), true);
        let at = egui::pos2(24.0, 12.0)
            + if index < 3 {
                egui::vec2(47.5 + index as f32 * 60.0, 290.5)
            } else {
                egui::vec2(238.0 + (index - 3) as f32 * 67.0, 73.0)
            };
        let mut ui_action = None;
        for frame in 0..3 {
            let events = if frame == 2 {
                vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                vec![]
            };
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 700.0),
                    )),
                    focused: true,
                    time: Some(frame as f64 * 0.1),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            ui_action = rack.show(ui, &patch, &openpatch_core::catalog(), 48000, 1);
                        });
                },
            );
        }
        let Some(crate::rack::RackAction::ManualClick { id, press, release }) = ui_action else {
            panic!("Actual pointer click must reach the host's deferred path");
        };
        assert_eq!(id, "seq");
        let press = serde_json::from_value::<ModuleAction>(serde_json::json!(press)).unwrap();
        let release = serde_json::from_value::<ModuleAction>(serde_json::json!(release)).unwrap();
        assert_eq!((press, release), expected_pair);
        let ticket = callback
            .performance
            .press_manual_with_ticket("seq", press, release)
            .unwrap();
        assert!(
            !ticket.ready_to_release(),
            "A GUI frame alone cannot release an unobserved press"
        );
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(ticket.ready_to_release());
        assert_eq!(
            callback
                .performance
                .module_control_state("seq")
                .unwrap()
                .pressed_mask,
            1 << index
        );
        let snapshot = callback.performance.snapshot();
        let state = &snapshot["modules"]["seq"]["state"];
        match index {
            0 => assert_ne!(state["step"], before.step),
            1 => assert_eq!(state["running"], false),
            2 => assert_eq!(state["step"], 1),
            _ => assert_eq!(state["gates"][index - 3], false),
        }
        drop(ticket);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback
                .performance
                .module_control_state("seq")
                .unwrap()
                .pressed_mask,
            0
        );
    }
}
#[test]
fn stock_seq3_cancelled_click_tickets_cannot_press_after_cancellation_retirement_or_backend_replacement(
) {
    for case in 0..4 {
        let patch = stock_seq3_transport_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        let ticket = callback
            .performance
            .press_manual_with_ticket("seq", ModuleAction::RunPress, ModuleAction::RunRelease)
            .unwrap();
        assert!(!ticket.ready_to_release());
        match case {
            0 => callback
                .performance
                .release_module_controls(&["seq".into()]),
            1 => {
                let mut bypass = patch.clone();
                bypass.nodes[0].bypassed = true;
                callback.performance.sync_patch(&bypass, 2);
                assert!(updates.push(sequence_graph(&callback, &bypass, 2)).is_ok());
            }
            2 => {
                callback.performance.sync_patch(&Patch::new_v2(), 2);
                callback.performance.sync_patch(&patch, 3);
                assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
            }
            _ => callback.performance.close(),
        }
        assert!(ticket.ready_to_release());
        drop(ticket);
        if case == 3 {
            let (mut fresh, _, _) = sequence_callback(&patch);
            without_heap_activity(|| fresh.render(&mut [0.0_f32; 64], 1));
            assert_eq!(
                fresh.performance.snapshot()["modules"]["seq"]["state"]["running"],
                true
            );
            continue;
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        while disposal.pop().is_ok() {}
        assert_eq!(
            callback
                .performance
                .module_control_state("seq")
                .unwrap()
                .pressed_mask,
            0
        );
        assert_eq!(
            callback.performance.snapshot()["modules"]["seq"]["state"]["running"],
            true,
            "Cancelled pending press must not become a ghost Run toggle"
        );
    }
}
#[test]
fn stock_seq3_dropped_sampled_click_cannot_accept_a_second_press_until_source_samples_low() {
    for (press, release) in [
        (ModuleAction::RunPress, ModuleAction::RunRelease),
        (
            ModuleAction::SeqGatePress(1),
            ModuleAction::SeqGateRelease(1),
        ),
    ] {
        let patch = stock_seq3_transport_patch();
        let (mut callback, _, _) = sequence_callback(&patch);
        callback.shared.playing.store(false, Ordering::SeqCst);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        let ticket = callback
            .performance
            .press_manual_with_ticket("seq", press, release)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(ticket.ready_to_release());
        drop(ticket);
        assert!(
            callback
                .performance
                .press_manual_with_ticket("seq", press, release)
                .is_err(),
            "Release+Press must not collapse into an un-rearmed HIGH sample"
        );
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback
                .performance
                .module_control_state("seq")
                .unwrap()
                .pressed_mask,
            0
        );
        let mut second = callback
            .performance
            .press_manual_with_ticket("seq", press, release)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let snapshot = callback.performance.snapshot();
        let state = &snapshot["modules"]["seq"]["state"];
        if press == ModuleAction::RunPress {
            assert_eq!(state["running"], true);
        } else {
            assert_eq!(state["gates"][0], true);
        }
        second.begin_release();
        assert!(!second.is_finished());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(second.is_finished());
        let before = callback.performance.snapshot()["queued"].clone();
        drop(second);
        assert_eq!(
            callback.performance.snapshot()["queued"],
            before,
            "An acknowledged ticket cannot post a duplicate stale release"
        );
    }
}
