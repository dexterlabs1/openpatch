//! Native exclusive binding policy, through the actual public engine API.
use openpatch_core::{Engine, Patch, PortFrame};
use serde_json::json;

fn patch(ramp: bool) -> Patch {
    let sequence = |id: &str, value: f32| {
        let mut points = vec![json!({"bar":0,"value":value})];
        if ramp {
            points.push(json!({"bar":1,"value":value+0.05}));
        }
        json!({"id":id,"kind":"rackmcp_sequencer","params":{"running":1},
            "rackmcp_sequence":{"data":{"patterns":[{"name":"A","length":16}],
                "lanes":[{"moduleId":"42","paramId":0,"points":points}]},
                "targets":[{"source_module_id":"42","source_param_id":0,
                    "module":"vca","parameter":"level"}]}})
    };
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[sequence("A",0.25),sequence("B",0.75),
            {"id":"vca","kind":"stock_vca","params":{"level":0.9}}],
        "output":"A","output_port":"pitch1"}))
    .unwrap()
}

fn tick(engine: &mut Engine) {
    engine.process_poly_block(&mut [PortFrame::default(); 32]);
}

fn value(engine: &Engine) -> f32 {
    engine
        .parameter_value(engine.resolve_parameter("vca", "level").unwrap())
        .unwrap()
}

fn manual(engine: &mut Engine, value: f32) {
    let gain = engine.resolve_parameter("vca", "level").unwrap();
    assert!(engine.set_parameter_value(gain, value));
}

fn reload(engine: &mut Engine, id: &str) {
    let handle = engine.resolve_module_state(id).unwrap();
    assert!(engine.reload_rackmcp_sequence_data(handle));
}

#[test]
fn cold_order_explicit_takeover_idempotent_reload_and_unowned_cache() {
    let mut engine = patch(false).compile().unwrap();
    tick(&mut engine);
    assert_eq!(value(&engine), 0.75, "last native node wins a cold graph");
    manual(&mut engine, 0.6);
    tick(&mut engine);
    assert_eq!(
        value(&engine),
        0.6,
        "idle displaced writer cannot undo manual value"
    );
    let a = engine.resolve_module_state("A").unwrap();
    assert!(engine.claim_rackmcp_sequence_targets(a));
    tick(&mut engine);
    assert_eq!(
        value(&engine),
        0.25,
        "unowned attempts must never acknowledge cache"
    );
    reload(&mut engine, "B");
    tick(&mut engine);
    assert_eq!(value(&engine), 0.75);
    manual(&mut engine, 0.6);
    reload(&mut engine, "B");
    tick(&mut engine);
    assert_eq!(
        value(&engine),
        0.75,
        "healthy incumbent reload retains its claim"
    );
    reload(&mut engine, "A");
    tick(&mut engine);
    assert_eq!(value(&engine), 0.25);
    manual(&mut engine, 0.6);
    assert!(engine.claim_rackmcp_sequence_targets(a));
    tick(&mut engine);
    assert_eq!(
        value(&engine),
        0.6,
        "claim-only preserves acknowledged cache"
    );
}

#[test]
fn deleting_winner_retains_vacancy_across_rebuilds_until_explicit_acquisition() {
    let mut patch = patch(false);
    let mut old = patch.compile().unwrap();
    tick(&mut old);
    patch.nodes.retain(|node| node.id != "B");
    for rate in [48000, 96000, 48000] {
        patch.sample_rate = rate;
        patch.nodes.reverse();
        let mut next = patch.compile().unwrap();
        next.adopt_state_from(&old);
        next.inherit_parameter_values(&old, &[], &[]);
        manual(&mut next, 0.6);
        tick(&mut next);
        assert_eq!(value(&next), 0.6, "deleting winner cannot revive A");
        old = next;
    }
    reload(&mut old, "A");
    tick(&mut old);
    assert_eq!(value(&old), 0.25);
}

#[test]
fn runtime_owner_survives_unrelated_edits_reorder_and_rate_handoff() {
    let mut patch = patch(true);
    let mut old = patch.compile().unwrap();
    reload(&mut old, "A");
    tick(&mut old);
    for rate in [48000, 96000, 44100] {
        patch.sample_rate = rate;
        patch.nodes.swap(0, 1);
        patch
            .nodes
            .iter_mut()
            .find(|node| node.id == "B")
            .unwrap()
            .params
            .insert("bpm".into(), 130.0);
        let mut next = patch.compile().unwrap();
        next.adopt_state_from(&old);
        next.inherit_parameter_values(&old, &[], &[]);
        tick(&mut next);
        assert!(
            value(&next) < 0.3,
            "live A claim overrides compiled node order"
        );
        old = next;
    }
}

#[test]
fn changed_data_binding_and_fresh_incarnation_acquire_after_restoration() {
    let mut patch = patch(true);
    let mut old = patch.compile().unwrap();
    tick(&mut old);
    patch.nodes[0].rackmcp_sequence.as_mut().unwrap().data.seed += 1;
    let mut changed = patch.compile().unwrap();
    changed.adopt_state_from(&old);
    changed.inherit_parameter_values(&old, &[], &[]);
    tick(&mut changed);
    assert!(value(&changed) < 0.3, "changed A data takes target from B");

    let mut fresh = patch.compile().unwrap();
    let b = fresh.resolve_module_state("B").unwrap();
    fresh.adopt_state_from_excluding(&changed, &[b]);
    fresh.inherit_parameter_values(&changed, &[], &[]);
    tick(&mut fresh);
    assert!(
        value(&fresh) > 0.7,
        "fresh B acquires after donor A is restored"
    );

    // Moving A away relinquishes its former set; moving it back acquires.
    patch.nodes[0].rackmcp_sequence.as_mut().unwrap().targets[0].module = "missing".into();
    let mut moved = patch.compile().unwrap();
    moved.adopt_state_from(&fresh);
    moved.inherit_parameter_values(&fresh, &[], &[]);
    tick(&mut moved);
    assert!(value(&moved) > 0.7);
    patch.nodes[0].rackmcp_sequence.as_mut().unwrap().targets[0].module = "vca".into();
    let mut rebound = patch.compile().unwrap();
    rebound.adopt_state_from(&moved);
    rebound.inherit_parameter_values(&moved, &[], &[]);
    tick(&mut rebound);
    assert!(
        value(&rebound) < 0.3,
        "changed binding acquires without changed source data"
    );
}

#[test]
fn missing_destination_keeps_claim_and_recreation_keeps_cache() {
    let mut patch = patch(false);
    let target = patch.nodes.pop().unwrap();
    let mut missing = patch.compile().unwrap();
    reload(&mut missing, "A");
    tick(&mut missing);
    patch.nodes.push(target.clone());
    let mut present = patch.compile().unwrap();
    present.adopt_state_from(&missing);
    tick(&mut present);
    assert_eq!(
        value(&present),
        0.25,
        "missing target retains A claim and unwritten cache"
    );
    patch.nodes.pop();
    let mut removed = patch.compile().unwrap();
    removed.adopt_state_from(&present);
    tick(&mut removed);
    patch.nodes.push(target);
    let mut recreated = patch.compile().unwrap();
    recreated.adopt_state_from(&removed);
    tick(&mut recreated);
    assert_eq!(
        value(&recreated),
        0.9,
        "successful cache survives target recreation"
    );
    reload(&mut recreated, "A");
    tick(&mut recreated);
    assert_eq!(value(&recreated), 0.25);
}

#[test]
fn claims_reject_foreign_and_non_sequence_handles() {
    let mut first = patch(false).compile().unwrap();
    let second = patch(false).compile().unwrap();
    let foreign = second.resolve_module_state("A").unwrap();
    let target = first.resolve_parameter("vca", "level").unwrap().module();
    assert!(!first.claim_rackmcp_sequence_targets(foreign));
    assert!(!first.reload_rackmcp_sequence_data(foreign));
    assert!(!first.claim_rackmcp_sequence_targets(target));
    tick(&mut first);
    assert_eq!(value(&first), 0.75);
}

#[test]
fn removed_winners_binding_never_reactivates_displaced_claim() {
    let mut patch = patch(false);
    let mut old = patch.compile().unwrap();
    tick(&mut old);
    patch.nodes[1]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .targets
        .clear();
    let mut unbound = patch.compile().unwrap();
    unbound.adopt_state_from(&old);
    unbound.inherit_parameter_values(&old, &[], &[]);
    manual(&mut unbound, 0.6);
    tick(&mut unbound);
    assert_eq!(value(&unbound), 0.6);
    let mut unrelated = patch.compile().unwrap();
    unrelated.adopt_state_from(&unbound);
    unrelated.inherit_parameter_values(&unbound, &[], &[]);
    tick(&mut unrelated);
    assert_eq!(value(&unrelated), 0.6);
    reload(&mut unrelated, "A");
    tick(&mut unrelated);
    assert_eq!(value(&unrelated), 0.25);
}

#[test]
fn native_key_aliases_have_one_lane_owner_and_lane_order_cold_policy() {
    let mut patch = patch(false);
    patch.nodes.retain(|node| node.id != "B");
    let state = patch.nodes[0].rackmcp_sequence.as_mut().unwrap();
    let mut lane = state.data.lanes[0].clone();
    lane.param_id = 1;
    lane.points[0].value = 0.5;
    state.data.lanes.push(lane);
    let mut target = state.targets[0].clone();
    target.source_param_id = 1;
    state.targets.push(target);
    let mut engine = patch.compile().unwrap();
    tick(&mut engine);
    assert_eq!(value(&engine), 0.5);
    manual(&mut engine, 0.6);
    tick(&mut engine);
    assert_eq!(
        value(&engine),
        0.6,
        "the aliased displaced lane never receives ownership"
    );
}
