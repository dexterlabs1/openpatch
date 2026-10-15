// Real audio callback + typed Learn transport; no window or physical device.
fn midi_gate_host_patch() -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"gate","kind":"midi_gate"},{"id":"cc","kind":"midi_cc"}],
        "output":"gate","output_port":"gate1"}))
    .unwrap()
}
fn gate_note(note: u8, velocity: u8) -> MidiEvent {
    MidiEvent::NoteOn {
        channel: 0,
        note,
        velocity,
    }
}
fn gate_note_off(note: u8) -> MidiEvent {
    MidiEvent::NoteOff {
        channel: 0,
        note,
        velocity: 0,
    }
}
fn gate_host_state(callback: &Callback) -> serde_json::Value {
    callback.performance.snapshot()["modules"]["gate"]["state"].clone()
}
fn gate_host_render(callback: &mut Callback, frames: usize) {
    let mut output = [0.0_f32; 1024];
    without_heap_activity(|| callback.render(&mut output[..frames], 1));
}
fn gate_host_publish(
    callback: &mut Callback,
    updates: &mut Producer<PreparedGraph>,
    disposal: &mut Consumer<PreparedGraph>,
    session: &openpatch::control::Session,
) {
    assert!(updates
        .push(sequence_graph(
            callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    for _ in 0..4 {
        gate_host_render(callback, 256);
        while disposal.pop().is_ok() {}
        if callback.graphs.active.revision == session.revision() {
            return;
        }
    }
    panic!("new graph did not publish");
}
#[test]
fn midi_gate_host_immediate_note_off_has_exact_minimum_pulse_on_the_event_sample() {
    let patch = midi_gate_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let mut ui = callback.midi.take_ui().unwrap();
    assert!(ui.send(3, gate_note(48, 127)));
    assert!(ui.send(3, gate_note_off(48)));
    let mut output = [0.0_f32; 80];
    without_heap_activity(|| callback.render(&mut output, 1));
    for (frame, value) in output.iter().enumerate() {
        assert_eq!(
            *value,
            if (3..51).contains(&frame) { 1.0 } else { 0.0 },
            "frame {frame}"
        );
    }
    assert_eq!(
        callback
            .performance
            .module_lights("gate")
            .unwrap()
            .brightness,
        [[0.0; 2]; 16]
    );
}
#[test]
fn midi_gate_host_cross_kind_completions_share_sequence_and_commit_each_mapping_once() {
    use openpatch_core::MidiLearnKind;
    let patch = midi_gate_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let mut hardware = callback.midi.take_hardware().unwrap();
    let mut ui = callback.midi.take_ui().unwrap();
    gate_host_render(&mut callback, 1);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    session
        .execute("learn_midi_cc", serde_json::json!({"id":"cc","cell":1}))
        .unwrap();
    session
        .execute("learn_midi_gate", serde_json::json!({"id":"gate","cell":1}))
        .unwrap();
    assert!(hardware.send(4, midi_cc_event(0, 7, 64)));
    // Zero velocity and off messages cannot learn, even when ordered before on.
    assert!(ui.send(2, gate_note(36, 0)));
    assert!(ui.send(3, gate_note_off(36)));
    assert!(ui.send(4, gate_note(36, 100)));
    gate_host_render(&mut callback, 6);
    let ready = callback.performance.midi_learn_ready();
    assert_eq!(ready.len(), 2);
    assert_eq!(
        (ready[0].kind, ready[1].kind),
        (MidiLearnKind::Cc, MidiLearnKind::Gate)
    );
    assert_eq!((ready[0].frame, ready[1].frame), (4, 4));
    assert!(ready[0].sequence < ready[1].sequence);
    assert_eq!(ready[0].backend, ready[1].backend);
    assert_ne!(ready[0].backend, 0);
    assert_eq!(ready[1].completion.after[0], 36);
    assert_eq!(ready[1].completion.after[12], -1);
    assert_eq!(
        callback
            .performance
            .module_lights("gate")
            .unwrap()
            .brightness[0],
        [1.0, 0.0]
    );
    assert_eq!(
        callback.performance.midi_cc_learn_ready().len(),
        1,
        "legacy peek cannot include Gate"
    );
    let mut wrong = ready[1].clone();
    wrong.kind = MidiLearnKind::Cc;
    assert!(callback.performance.acknowledge_midi_learn(&wrong).is_err());
    wrong = ready[1].clone();
    wrong.backend += 1;
    assert!(callback.performance.acknowledge_midi_learn(&wrong).is_err());
    assert_eq!(callback.performance.midi_learn_ready().len(), 2);
    let rev = session.revision();
    session.reconcile_midi_learn().unwrap();
    assert_eq!(session.revision(), rev + 2);
    session.reconcile_midi_cc_learn().unwrap();
    assert_eq!(
        session.revision(),
        rev + 2,
        "compatibility alias must not double-commit"
    );
    assert!(callback.performance.midi_learn_ready().is_empty());
    assert_eq!(session.patch().nodes[0].params["note1"], 36.0);
    assert_eq!(session.patch().nodes[1].params["cc1"], 7.0);
}
#[test]
fn midi_gate_host_remapping_keeps_cell_gate_until_panic_and_panic_works_with_full_queue() {
    use openpatch_core::MidiLearnKind;
    let patch = midi_gate_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut ui = callback.midi.take_ui().unwrap();
    assert!(ui.send(0, gate_note(48, 127)));
    gate_host_render(&mut callback, 64);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    session
        .execute(
            "set_parameter",
            serde_json::json!({"id":"gate","parameter":"note1","value":60}),
        )
        .unwrap();
    gate_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert!(ui.send(callback.frame_clock, gate_note_off(48)));
    gate_host_render(&mut callback, 1);
    assert_eq!(
        callback
            .performance
            .module_lights("gate")
            .unwrap()
            .brightness[0],
        [1.0, 0.0],
        "cell-owned held gate survives old note-off"
    );
    callback
        .performance
        .learn_midi(MidiLearnKind::Gate, "gate", 1)
        .unwrap();
    // Fill with valid no-history actions on the same module; Panic must fall
    // back to its emergency epoch instead of failing or allocating in audio.
    for _ in 0..EVENT_CAPACITY {
        callback
            .performance
            .trigger("gate", ModuleAction::MidiPanic)
            .unwrap();
    }
    let fallback = callback
        .performance
        .trigger("gate", ModuleAction::MidiPanic)
        .unwrap();
    assert_eq!(fallback["emergency_release"], true);
    gate_host_render(&mut callback, 1);
    assert_eq!(
        callback
            .performance
            .module_lights("gate")
            .unwrap()
            .brightness,
        [[0.0; 2]; 16]
    );
    assert_eq!(
        gate_host_state(&callback)["learning"],
        serde_json::Value::Null
    );
    assert!(!callback.performance.midi_learning_active());
    assert!(
        callback.performance.snapshot()["emergency_release_applied"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(ui.send(callback.frame_clock, gate_note(61, 127)));
    gate_host_render(&mut callback, 1);
    assert!(callback.performance.midi_learn_ready().is_empty());
    assert_eq!(gate_host_state(&callback)["assignments"][1], 49);
}
#[test]
fn midi_gate_host_ready_survives_reset_panic_and_close_without_heap_activity() {
    use openpatch_core::MidiLearnKind;
    let patch = midi_gate_host_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    let mut ui = callback.midi.take_ui().unwrap();
    callback
        .performance
        .learn_midi(MidiLearnKind::Gate, "gate", 0)
        .unwrap();
    assert!(ui.send(0, gate_note(67, 1)));
    gate_host_render(&mut callback, 1);
    let before = callback.performance.midi_learn_ready();
    assert_eq!(before.len(), 1);
    callback.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
    gate_host_render(&mut callback, 1);
    callback
        .performance
        .trigger("gate", ModuleAction::MidiPanic)
        .unwrap();
    gate_host_render(&mut callback, 1);
    assert_eq!(
        callback.performance.midi_learn_ready()[0].completion,
        before[0].completion
    );
    callback.performance.close();
    gate_host_render(&mut callback, 1);
    assert_eq!(
        callback.performance.midi_learn_ready()[0].completion,
        before[0].completion
    );
}
#[test]
fn midi_gate_host_old_cc_ticket_and_receipt_cannot_target_same_id_gate_replacement() {
    use openpatch_core::MidiLearnKind;
    let mut patch = midi_gate_host_patch();
    patch.nodes[0].kind = "midi_cc".into();
    patch.output_port = "out1".into();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut ui = callback.midi.take_ui().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    callback.performance.learn_midi_cc("gate", 0).unwrap();
    assert!(ui.send(0, midi_cc_event(0, 74, 1)));
    gate_host_render(&mut callback, 1);
    let old = callback.performance.midi_cc_learn_ready().remove(0);
    session.reconcile_midi_learn().unwrap();
    session
        .replace_patch(midi_gate_host_patch(), Some(session.revision()))
        .unwrap();
    callback
        .performance
        .learn_midi(MidiLearnKind::Gate, "gate", 0)
        .unwrap();
    gate_host_publish(&mut callback, &mut updates, &mut disposal, &session);
    assert!(ui.send(callback.frame_clock, gate_note(72, 100)));
    gate_host_render(&mut callback, 1);
    let current = callback.performance.midi_learn_ready();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].kind, MidiLearnKind::Gate);
    assert!(callback
        .performance
        .acknowledge_midi_cc_learn(&old)
        .is_err());
    assert_eq!(callback.performance.midi_learn_ready().len(), 1);
    session.reconcile_midi_learn().unwrap();
    assert_eq!(session.patch().nodes[0].params["note1"], 72.0);
}
#[test]
fn midi_gate_host_coalesced_bypass_resume_clears_held_gates_short_timers_and_armed_learn() {
    use openpatch_core::MidiLearnKind;
    for held in [true, false] {
        let patch = midi_gate_host_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let mut ui = callback.midi.take_ui().unwrap();
        assert!(ui.send(0, gate_note(48, 127)));
        if !held {
            assert!(ui.send(0, gate_note_off(48)));
        }
        gate_host_render(&mut callback, 1);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .learn_midi(MidiLearnKind::Gate, "gate", 1)
            .unwrap();
        gate_host_render(&mut callback, 1);
        assert_eq!(gate_host_state(&callback)["learning"]["cell"], 2);
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"gate","bypassed":true}),
            )
            .unwrap();
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"gate","bypassed":false}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        gate_host_render(&mut callback, 1);
        let lights = callback.performance.module_lights("gate").unwrap();
        assert_eq!(
            lights.brightness, [[0.0; 2]; 16],
            "coalesced bypass must clear held={held}"
        );
        assert_eq!(
            gate_host_state(&callback)["learning"],
            serde_json::Value::Null
        );
        assert!(callback.performance.midi_learn_ready().is_empty());
        while disposal.pop().is_ok() {}
    }
}
#[test]
fn midi_gate_host_learn_requested_after_bypass_resume_is_not_cancelled_by_the_old_fence() {
    let patch = midi_gate_host_patch();
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    let mut ui = callback.midi.take_ui().unwrap();
    assert!(ui.send(0, gate_note(48, 127)));
    gate_host_render(&mut callback, 64);
    let workspace = tempfile::tempdir().unwrap();
    let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
    session.replace_patch(patch, Some(0)).unwrap();
    session
        .set_live_module_events(Some(callback.performance.clone()))
        .unwrap();
    session
        .execute(
            "set_bypass",
            serde_json::json!({"id":"gate","bypassed":true}),
        )
        .unwrap();
    session
        .execute(
            "set_bypass",
            serde_json::json!({"id":"gate","bypassed":false}),
        )
        .unwrap();
    // This accepted Learn belongs to the resumed configuration, after the fence.
    session
        .execute("learn_midi_gate", serde_json::json!({"id":"gate","cell":2}))
        .unwrap();
    assert!(updates
        .push(sequence_graph(
            &callback,
            session.patch(),
            session.revision()
        ))
        .is_ok());
    gate_host_render(&mut callback, 1);
    assert_eq!(
        callback
            .performance
            .module_lights("gate")
            .unwrap()
            .brightness,
        [[0.0; 2]; 16]
    );
    assert_eq!(
        gate_host_state(&callback)["learning"]["cell"],
        2,
        "old bypass fence cannot cancel new Learn"
    );
    assert!(ui.send(callback.frame_clock, gate_note(72, 127)));
    gate_host_render(&mut callback, 1);
    let ready = callback.performance.midi_learn_ready();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].completion.after[1], 72);
    while disposal.pop().is_ok() {}
}
#[test]
fn midi_gate_host_delayed_module_panic_preserves_only_later_learn_requests() {
    use openpatch_core::MidiLearnKind;
    for full in [false, true] {
        for learn_after in [false, true] {
            let patch = midi_gate_host_patch();
            let (mut callback, _, _) = sequence_callback(&patch);
            let mut ui = callback.midi.take_ui().unwrap();
            assert!(ui.send(0, gate_note(48, 127)));
            gate_host_render(&mut callback, 64);
            if !learn_after {
                callback
                    .performance
                    .learn_midi(MidiLearnKind::Gate, "gate", 1)
                    .unwrap();
            }
            if full {
                for _ in 0..EVENT_CAPACITY {
                    callback
                        .performance
                        .trigger("gate", ModuleAction::MidiPanic)
                        .unwrap();
                }
            }
            let receipt = callback
                .performance
                .trigger("gate", ModuleAction::MidiPanic)
                .unwrap();
            assert_eq!(receipt["emergency_release"] == true, full);
            if learn_after {
                callback
                    .performance
                    .learn_midi(MidiLearnKind::Gate, "gate", 1)
                    .unwrap();
            }
            gate_host_render(&mut callback, 1);
            assert_eq!(
                callback
                    .performance
                    .module_lights("gate")
                    .unwrap()
                    .brightness,
                [[0.0; 2]; 16]
            );
            assert_eq!(
                gate_host_state(&callback)["learning"]["cell"].as_u64(),
                learn_after.then_some(2),
                "full={full} learn_after={learn_after}"
            );
            assert!(ui.send(callback.frame_clock, gate_note(72, 127)));
            gate_host_render(&mut callback, 1);
            let ready = callback.performance.midi_learn_ready();
            assert_eq!(
                ready.len(),
                usize::from(learn_after),
                "full={full} learn_after={learn_after}"
            );
            if learn_after {
                assert_eq!(ready[0].completion.after[1], 72);
            }
        }
    }
}
