use super::*;
use openpatch_core::MidiEvent;

fn setup() -> (Arc<Performance>, EventReader, Patch, Engine, ModulePlan) {
    let patch: Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"cc","kind":"midi_cc"}],"output":"cc","output_port":"out1"})).unwrap();
    let engine = patch.compile().unwrap();
    let (performance, reader, _) = Performance::new(48_000);
    performance.sync_patch(&patch, 1);
    let plan = performance.plan(&patch, &engine, 1).unwrap();
    (performance, reader, patch, engine, plan)
}
fn cc(controller: u8, value: u8) -> MidiEvent {
    MidiEvent::ControlChange {
        channel: 0,
        controller,
        value,
    }
}
fn event(
    reader: &EventReader,
    engine: &mut Engine,
    plan: &mut ModulePlan,
    revision: u64,
    frame: u64,
    event: MidiEvent,
) {
    reader.before_midi_cc(engine, plan, revision);
    engine.handle_midi(event);
    reader.after_midi_cc(engine, plan, frame, event == MidiEvent::Panic);
}
fn mapping(engine: &Engine) -> [i16; 16] {
    let SavedModuleState::MidiCc { assignments, .. } = engine
        .module_saved_state(engine.resolve_module_state("cc").unwrap())
        .unwrap()
    else {
        unreachable!()
    };
    assignments
}
#[test]
fn durable_completion_is_peeked_until_exact_ack_and_survives_close() {
    let (p, r, _, mut e, mut plan) = setup();
    let arm = p.learn_midi_cc("cc", 0).unwrap();
    event(&r, &mut e, &mut plan, 1, 17, cc(45, 1));
    let ready = p.midi_cc_learn_ready();
    assert_eq!(ready.len(), 1);
    let receipt = &ready[0];
    assert_eq!(receipt.completion.serial, arm["serial"].as_u64().unwrap());
    assert_eq!(receipt.frame, 17);
    assert_eq!(receipt.completion.before[0], 1);
    assert_eq!(receipt.completion.after[0], 45);
    assert!(p.cancel_midi_cc_learn("cc").is_err());
    assert!(p.learn_midi_cc("cc", 1).is_err());
    p.close();
    assert_eq!(p.midi_cc_learn_ready()[0].completion, receipt.completion);
    let mut stale = receipt.clone();
    stale.epoch += 1;
    assert!(p.acknowledge_midi_cc_learn(&stale).is_err());
    assert_eq!(p.midi_cc_learn_ready().len(), 1);
    p.acknowledge_midi_cc_learn(receipt).unwrap();
    assert!(!p.midi_cc_learning_active());
    assert!(p.midi_cc_learn_ready().is_empty());
}
#[test]
fn processing_wins_cancel_close_but_nonqualifying_event_returns_armed() {
    let (p, r, _, mut e, mut plan) = setup();
    p.learn_midi_cc("cc", 0).unwrap();
    event(&r, &mut e, &mut plan, 1, 0, cc(45, 0));
    assert!(p.midi_cc_learning_active());
    assert!(!p.midi_cc_learning_uncommitted());
    r.before_midi_cc(&mut e, &mut plan, 1);
    assert!(p.midi_cc_learning_uncommitted());
    assert!(p.prepare_midi_cc_edit(&["cc".into()]).is_err());
    assert!(p.cancel_midi_cc_learn("cc").is_err());
    p.close();
    e.handle_midi(cc(45, 1));
    r.after_midi_cc(&mut e, &mut plan, 1, false);
    assert_eq!(p.midi_cc_learn_ready().len(), 1);
    assert_eq!(mapping(&e)[0], 45);
}
#[test]
fn cancellation_between_sync_and_event_never_leaves_unreserved_core_learn() {
    let (p, r, _, mut e, mut plan) = setup();
    p.learn_midi_cc("cc", 0).unwrap();
    r.sync_midi_cc(&mut e, &mut plan, 1);
    assert!(e
        .midi_cc_learning(e.resolve_module_state("cc").unwrap())
        .is_some());
    p.cancel_midi_cc_learn("cc").unwrap();
    event(&r, &mut e, &mut plan, 1, 3, cc(88, 12));
    assert_eq!(mapping(&e)[0], 1);
    assert!(p.midi_cc_learn_ready().is_empty());
    assert!(e
        .take_midi_cc_learn_completion(e.resolve_module_state("cc").unwrap())
        .is_none());
}
#[test]
fn armed_close_panic_and_transport_reset_cancel_without_losing_ready() {
    let (p, r, _, mut e, mut plan) = setup();
    p.learn_midi_cc("cc", 0).unwrap();
    event(&r, &mut e, &mut plan, 1, 1, MidiEvent::Panic);
    assert!(!p.midi_cc_learning_active());
    p.learn_midi_cc("cc", 0).unwrap();
    r.cancel_midi_cc(&mut e, &mut plan);
    assert!(!p.midi_cc_learning_active());
    p.learn_midi_cc("cc", 0).unwrap();
    event(&r, &mut e, &mut plan, 1, 2, cc(30, 2));
    r.cancel_midi_cc(&mut e, &mut plan);
    assert_eq!(p.midi_cc_learn_ready().len(), 1);
    p.close();
    assert_eq!(p.midi_cc_learn_ready().len(), 1);
}
#[test]
fn stale_revision_configuration_instance_and_backend_cannot_complete_or_ack() {
    let (p, r, mut patch, mut e, mut oldplan) = setup();
    p.learn_midi_cc("cc", 0).unwrap();
    p.prepare_midi_cc_edit(&["cc".into()]).unwrap();
    patch.nodes[0].params.insert("smooth".into(), 0.0);
    p.sync_patch(&patch, 2);
    p.learn_midi_cc("cc", 0).unwrap();
    event(&r, &mut e, &mut oldplan, 1, 1, cc(90, 1));
    assert_eq!(mapping(&e)[0], 1);
    let mut current = patch.compile().unwrap();
    let mut currentplan = p.plan(&patch, &current, 2).unwrap();
    event(&r, &mut current, &mut currentplan, 2, 2, cc(90, 2));
    let receipt = p.midi_cc_learn_ready().remove(0);
    let (other, otherreader, _, mut otherengine, mut otherplan) = setup();
    other.learn_midi_cc("cc", 0).unwrap();
    event(
        &otherreader,
        &mut otherengine,
        &mut otherplan,
        1,
        2,
        cc(90, 2),
    );
    assert!(other.acknowledge_midi_cc_learn(&receipt).is_err());
    p.acknowledge_midi_cc_learn(&receipt).unwrap();
    p.learn_midi_cc("cc", 1).unwrap();
    p.retire_modules(&["cc".into()]);
    p.sync_patch(&patch, 3);
    let mut replacement = patch.compile().unwrap();
    let mut newplan = p.plan(&patch, &replacement, 3).unwrap();
    p.learn_midi_cc("cc", 0).unwrap();
    event(&r, &mut current, &mut currentplan, 3, 3, cc(91, 3));
    assert!(p.midi_cc_learn_ready().is_empty());
    event(&r, &mut replacement, &mut newplan, 3, 4, cc(91, 3));
    assert_eq!(p.midi_cc_learn_ready().len(), 1);
}
#[test]
fn reconciliation_authorizes_ready_map_without_changing_epoch_and_undo_preserves_cc_motion() {
    let (p, r, _, mut e, mut plan) = setup();
    let mut session = openpatch::control::Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute("add_module", json!({"id":"cc","kind":"midi_cc"}))
        .unwrap();
    session.set_live_module_events(Some(p.clone())).unwrap();
    session
        .execute("learn_midi_cc", json!({"id":"cc","cell":1}))
        .unwrap();
    event(&r, &mut e, &mut plan, 1, 10, cc(45, 20));
    let before = p.midi_cc_learn_ready().remove(0);
    session.reconcile_midi_cc_learn().unwrap();
    assert!(p.midi_cc_learn_ready().is_empty());
    let cc_registry = p.control.lock().unwrap();
    assert_eq!(cc_registry.cc_learns["cc"].epoch, before.epoch);
    drop(cc_registry);
    assert_eq!(session.patch().nodes[0].params["cc1"], 45.0);
    event(&r, &mut e, &mut plan, 1, 11, cc(45, 90));
    session.execute("undo", json!({})).unwrap();
    let mut undoengine = session.patch().compile().unwrap();
    let mut undoplan = p
        .plan(session.patch(), &undoengine, session.revision())
        .unwrap();
    undoplan.adopt_state(&mut undoengine, &plan, &e);
    undoplan.load_saved_pending(&mut undoengine, None);
    let SavedModuleState::MidiCc {
        assignments,
        values,
    } = undoengine
        .module_saved_state(undoengine.resolve_module_state("cc").unwrap())
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(assignments[0], 1);
    assert_eq!(values[45], 90);
}
#[test]
fn concurrent_cancel_and_event_have_one_atomic_winner() {
    for _ in 0..64 {
        let (p, r, _, mut e, mut plan) = setup();
        p.learn_midi_cc("cc", 0).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let barrier_audio = barrier.clone();
            let audio = scope.spawn(move || {
                barrier_audio.wait();
                event(&r, &mut e, &mut plan, 1, 12, cc(99, 1));
                mapping(&e)
            });
            barrier.wait();
            let cancellation = p.cancel_midi_cc_learn("cc");
            let mappings = audio.join().unwrap();
            if cancellation.is_ok() {
                assert_eq!(mappings[0], 1);
                assert!(p.midi_cc_learn_ready().is_empty());
            } else {
                assert_eq!(mappings[0], 99);
                assert_eq!(p.midi_cc_learn_ready().len(), 1);
            }
        });
    }
}

#[test]
fn panic_and_transport_reset_cancel_tickets_waiting_for_future_graph_adoption() {
    for panic in [false, true] {
        let (p, r, mut patch, mut e, mut plan) = setup();
        patch
            .nodes
            .push(serde_json::from_value(json!({"id":"future","kind":"midi_cc"})).unwrap());
        p.sync_patch(&patch, 2);
        p.learn_midi_cc("future", 0).unwrap();
        if panic {
            event(&r, &mut e, &mut plan, 1, 1, MidiEvent::Panic);
        } else {
            r.cancel_midi_cc(&mut e, &mut plan);
        }
        assert!(!p.midi_cc_learning_active());
    }
}
