use super::*;
use openpatch_core::{InputLifecycleTransform, OutputLifecycleTransform};

fn make_session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.replace_patch(serde_json::from_value(json!({
        "version":2,"nodes":[
            {"id":"a","kind":"constant","params":{"value":2}},
            {"id":"b","kind":"constant","params":{"value":3}},
            {"id":"sink","kind":"gain"}],
        "cables":[{"id":"wire","from":"a","to":"sink","input":"signal","input_order":0,"output_order":0}],
        "output":"sink"
    })).unwrap(),None).unwrap();
    session.discard_audio_lifecycle_through_current();
    session
}
fn disconnect() -> Value {
    json!({"tool":"disconnect","arguments":{"cable_id":"wire"}})
}
fn connect() -> Value {
    json!({"tool":"connect","arguments":{"cable_id":"wire","from":"a","to":"sink","input":"signal"}})
}
fn bypass(value: bool) -> Value {
    json!({"tool":"set_bypass","arguments":{"id":"a","bypassed":value}})
}
fn batch(session: &mut Session, edits: Vec<Value>) {
    session
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
}
fn take_delta(session: &mut Session) -> HostLifecycleDelta {
    let ticket = session.audio_snapshot().unwrap();
    assert!(session.acknowledge_audio_snapshot(ticket.id));
    ticket.delta.clone()
}
fn output(delta: &HostLifecycleDelta, id: &str, initial: u8) -> (u8, [f32; 16]) {
    let mut count = initial;
    let mut values = std::array::from_fn(|lane| (lane + 1) as f32);
    delta
        .modules
        .get(id)
        .and_then(|m| m.outputs.get("out"))
        .copied()
        .unwrap_or_else(OutputLifecycleTransform::identity)
        .apply(&mut count, &mut values)
        .unwrap();
    (count, values)
}

#[test]
fn disconnect_reconnect_in_one_batch_keeps_input_clear_with_same_final_dsp_topology() {
    let mut session = make_session();
    let mut before = session.patch.reference_view();
    batch(&mut session, vec![disconnect(), connect()]);
    let mut after = session.patch.reference_view();
    // Reconnection advances cable presentation ranks. All other patch fields,
    // including cable identity/endpoints, nodes and selected output, must match.
    for view in [&mut before, &mut after] {
        for cable in view["cables"].as_array_mut().unwrap() {
            let cable = cable.as_object_mut().unwrap();
            cable.remove("input_order");
            cable.remove("output_order");
        }
    }
    assert_eq!(after, before);
    let delta = take_delta(&mut session);
    assert_eq!(
        delta.modules["sink"].inputs["signal"].effect(),
        (true, Some(1))
    );
    assert_eq!(output(&delta, "a", 7).0, 1);
    assert_eq!(output(&delta, "a", 7).1[0], 1.);
}

#[test]
fn partial_stack_removal_is_not_an_input_disconnect() {
    let mut session = make_session();
    session
        .execute(
            "connect",
            json!({"cable_id":"other","from":"b","to":"sink","input":"signal","mode":"stack"}),
        )
        .unwrap();
    take_delta(&mut session);
    session
        .execute("disconnect", json!({"cable_id":"wire"}))
        .unwrap();
    let delta = take_delta(&mut session);
    assert!(!delta.modules.contains_key("sink"));
    assert_eq!(output(&delta, "a", 7).0, 0);
}

#[test]
fn selected_render_output_is_an_additional_physical_consumer() {
    let mut session = make_session();
    session.execute("set_output", json!({"id":"a"})).unwrap();
    let selected = take_delta(&mut session);
    assert!(!selected.modules.contains_key("a"));
    session
        .execute("disconnect", json!({"cable_id":"wire"}))
        .unwrap();
    let removed = take_delta(&mut session);
    assert!(!removed.modules.contains_key("a"));
    session.execute("set_output", json!({"id":"sink"})).unwrap();
    assert_eq!(output(&take_delta(&mut session), "a", 7).0, 0);
}

#[test]
fn bypass_event_order_distinguishes_disconnected_and_reconnected_prefixes() {
    let mut session = make_session();
    batch(&mut session, vec![disconnect(), bypass(true), connect()]);
    let delta = take_delta(&mut session);
    let (count, values) = output(&delta, "a", 16);
    assert_eq!(count, 1);
    assert_eq!(values[0], 1.);
    assert_eq!(values[15], 16.);
    let mut session = make_session();
    batch(&mut session, vec![bypass(true), disconnect(), connect()]);
    let (count, values) = output(&take_delta(&mut session), "a", 16);
    assert_eq!(count, 1);
    assert!(values.iter().all(|v| v.to_bits() == 0));
}

#[test]
fn same_bypass_assignment_and_numeric_edits_have_no_port_effect() {
    let mut session = make_session();
    batch(&mut session, vec![bypass(false), bypass(false)]);
    assert!(take_delta(&mut session).is_empty());
    session
        .execute(
            "set_parameter",
            json!({"id":"a","parameter":"value","value":4}),
        )
        .unwrap();
    assert!(take_delta(&mut session).is_empty());
    batch(
        &mut session,
        vec![bypass(true), bypass(true), bypass(false)],
    );
    assert!(output(&take_delta(&mut session), "a", 8).1[..8]
        .iter()
        .all(|v| v.to_bits() == 0));
}

#[test]
fn failed_and_preview_batches_do_not_publish_partial_lifecycle() {
    let mut session = make_session();
    let before = session.snapshot();
    let edits = vec![
        disconnect(),
        json!({"tool":"set_parameter","arguments":{"id":"missing","parameter":"value","value":1}}),
    ];
    assert!(session
        .execute("apply_edits", json!({"edits":edits}))
        .is_err());
    assert_eq!(session.snapshot(), before);
    assert!(take_delta(&mut session).is_empty());
    session
        .execute("preview_edits", json!({"edits":[disconnect(),connect()]}))
        .unwrap();
    assert_eq!(session.snapshot(), before);
    assert!(take_delta(&mut session).is_empty());
    assert!(session
        .execute(
            "set_bypass",
            json!({"id":"a","bypassed":true,"expected_revision":0})
        )
        .is_err());
    assert!(take_delta(&mut session).is_empty());
}

#[test]
fn retired_recreated_id_discards_earlier_transforms_and_keeps_later_connections() {
    let mut session = make_session();
    batch(
        &mut session,
        vec![
            bypass(true),
            json!({"tool":"remove_module","arguments":{"id":"a"}}),
            json!({"tool":"add_module","arguments":{"id":"a","kind":"constant","params":{"value":2}}}),
            connect(),
        ],
    );
    let delta = take_delta(&mut session);
    assert!(delta.modules["a"].reset);
    let (count, values) = output(&delta, "a", 16);
    assert_eq!(count, 1);
    assert_eq!(values[0], 1.);
    assert_eq!(values[15], 16.);
    assert_eq!(
        delta.modules["sink"].inputs["signal"].effect(),
        (true, Some(1))
    );
}

#[test]
fn undo_redo_replay_reverse_transition_intent_not_an_inverse_clear() {
    let mut session = make_session();
    batch(&mut session, vec![bypass(true), disconnect()]);
    let forward = take_delta(&mut session);
    assert_eq!(output(&forward, "a", 16).0, 0);
    assert!(output(&forward, "a", 16).1.iter().all(|v| v.to_bits() == 0));
    session.execute("undo", json!({})).unwrap();
    let reverse = take_delta(&mut session);
    let (count, values) = output(&reverse, "a", 0);
    assert_eq!(count, 1);
    assert_eq!(values[0].to_bits(), 0);
    assert_eq!(values[1], 2.);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(take_delta(&mut session), forward);
}

#[test]
fn undo_redo_same_patch_transaction_retains_clear_intent() {
    let mut session = make_session();
    batch(&mut session, vec![disconnect(), connect()]);
    take_delta(&mut session);
    for command in ["undo", "redo", "undo"] {
        session.execute(command, json!({})).unwrap();
        assert_eq!(
            take_delta(&mut session).modules["sink"].inputs["signal"].effect(),
            (true, Some(1))
        );
    }
}

#[test]
fn replace_new_document_and_kind_change_fence_incarnations() {
    let mut session = make_session();
    let patch = session.patch.clone();
    session.replace_patch(patch.clone(), None).unwrap();
    assert!(take_delta(&mut session).modules.values().all(|m| m.reset));
    session.execute("undo", json!({})).unwrap();
    assert!(take_delta(&mut session).modules.values().all(|m| m.reset));
    session.new_document(patch, None).unwrap();
    assert!(!session.can_undo());
    assert!(take_delta(&mut session).modules["a"].reset);
    batch(
        &mut session,
        vec![
            json!({"tool":"remove_module","arguments":{"id":"a"}}),
            json!({"tool":"add_module","arguments":{"id":"a","kind":"gain"}}),
            connect(),
        ],
    );
    assert!(take_delta(&mut session).modules["a"].reset);
}

#[test]
fn sealed_ticket_retries_keep_exact_patch_banks_revision_and_leave_new_tail() {
    let mut session = make_session();
    session
        .execute("disconnect", json!({"cable_id":"wire"}))
        .unwrap();
    let first = session.audio_snapshot().unwrap();
    let old_patch = first.patch.reference_view();
    let old_revision = first.revision;
    batch(&mut session, vec![connect(), bypass(true)]);
    let retry = session.audio_snapshot().unwrap();
    assert!(Arc::ptr_eq(&first, &retry));
    assert!(Arc::ptr_eq(&first.banks, &retry.banks));
    assert_eq!(retry.patch.reference_view(), old_patch);
    assert_eq!(retry.revision, old_revision);
    assert!(session.revision() > old_revision);
    assert!(!session.acknowledge_audio_snapshot(first.id + 1));
    assert!(Arc::ptr_eq(&first, &session.audio_snapshot().unwrap()));
    assert!(session.acknowledge_audio_snapshot(first.id));
    let second = session.audio_snapshot().unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(second.revision, session.revision());
    assert_eq!(
        second.delta.modules["sink"].inputs["signal"].effect(),
        (false, Some(1))
    );
    let combined = first.delta.clone().then(second.delta.clone());
    assert_eq!(
        combined.modules["sink"].inputs["signal"].effect(),
        (true, Some(1))
    );
    assert_eq!(output(&combined, "a", 16).1[0].to_bits(), 0);
    assert_eq!(output(&combined, "a", 16).1[1], 2.);
    assert!(!session.acknowledge_audio_snapshot(first.id));
    assert!(Arc::ptr_eq(&second, &session.audio_snapshot().unwrap()));
}

#[test]
fn cloned_snapshot_has_independent_publication_and_bootstrap_invalidates_old_ticket() {
    let mut session = make_session();
    batch(&mut session, vec![disconnect(), connect()]);
    let ticket = session.audio_snapshot().unwrap();
    let mut clone = session.clone();
    let cloned = clone.audio_snapshot().unwrap();
    assert_ne!(ticket.id, cloned.id);
    assert!(cloned.delta.is_empty());
    assert!(!clone.acknowledge_audio_snapshot(ticket.id));
    assert!(Arc::ptr_eq(&ticket, &session.audio_snapshot().unwrap()));
    batch(&mut session, vec![bypass(true)]);
    session.discard_audio_lifecycle_through_current();
    assert!(!session.acknowledge_audio_snapshot(ticket.id));
    let fresh = session.audio_snapshot().unwrap();
    assert_eq!(fresh.revision, session.revision());
    assert!(fresh.delta.is_empty());
    assert_ne!(fresh.id, ticket.id);
}

#[test]
fn composition_reset_fence_discards_earlier_but_not_later_port_operations() {
    let mut earlier = HostLifecycleDelta::default();
    earlier
        .modules
        .entry("m".into())
        .or_default()
        .inputs
        .insert("x".into(), InputLifecycleTransform::disconnect());
    let mut retirement = HostLifecycleDelta::default();
    retirement.modules.entry("m".into()).or_default().reset = true;
    let mut later = HostLifecycleDelta::default();
    later
        .modules
        .entry("m".into())
        .or_default()
        .inputs
        .insert("x".into(), InputLifecycleTransform::connect());
    earlier.append(&retirement);
    earlier.append(&later);
    assert!(earlier.modules["m"].reset);
    assert_eq!(earlier.modules["m"].inputs["x"].effect(), (false, Some(1)));
}

#[test]
fn unpublished_transient_ids_and_invalid_port_names_do_not_grow_pending_journal() {
    let mut session = make_session();
    for index in 0..32 {
        let id = format!("ephemeral{index}");
        batch(
            &mut session,
            vec![
                json!({"tool":"add_module","arguments":{"id":id,"kind":"constant"}}),
                json!({"tool":"remove_module","arguments":{"id":id}}),
                json!({"tool":"connect","arguments":{"cable_id":"transient","from":"a","to":"sink","input":format!("not_a_port_{index}")}}),
                json!({"tool":"disconnect","arguments":{"cable_id":"transient"}}),
            ],
        );
        assert!(session.host_lifecycle.tail.is_empty());
    }
    session
        .execute("disconnect", json!({"cable_id":"wire"}))
        .unwrap();
    let sealed = session.audio_snapshot().unwrap();
    session.execute("remove_module", json!({"id":"a"})).unwrap();
    assert!(session.host_lifecycle.tail.modules["a"].reset);
    assert!(session.acknowledge_audio_snapshot(sealed.id));
    assert!(session.host_lifecycle.tail.modules["a"].reset);
}
