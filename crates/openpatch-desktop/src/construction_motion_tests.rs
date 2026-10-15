use super::*;
use serde_json::json;
fn patch(value: Option<f32>) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"dc","kind":"constant","params":value.map(|v|json!({"value":v})).unwrap_or(json!({}))}]})).unwrap()
}
fn receipt(previous: u64, revision: u64, old: f32, new: f32) -> Value {
    json!({"plan_id":7,"completed_steps":1,"last_revision":revision,"current_revision":revision,"last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":7,"step_index":0,"before_revision":previous,"revision":revision,"label":"Adjust DC","new_modules":[],"new_cables":[],"parameter_changes":[{"id":"dc","kind":"constant","parameter":"value","old_effective":old,"new_effective":new}]}})
}
fn observe(
    t: &mut Tracker,
    before: &Patch,
    after: &Patch,
    previous: u64,
    revision: u64,
    snapshot: &Value,
    now: Instant,
) {
    t.observe(
        before,
        after,
        previous,
        revision,
        snapshot,
        &openpatch_core::catalog(),
        now,
    );
}
#[test]
fn receipt_default_values_dedup_and_hidden_window_expiry_are_exact() {
    let now = Instant::now();
    let mut t = Tracker::default();
    let before = patch(None);
    let after = patch(Some(3.0));
    let snapshot = receipt(9, 10, 0.0, 3.0);
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert_eq!(t.parameters.len(), 1);
    assert_eq!(t.parameters[0].old, 0.0);
    assert_eq!(t.focus.len(), 1);
    observe(
        &mut t,
        &before,
        &after,
        9,
        10,
        &snapshot,
        now + Duration::from_millis(180),
    );
    assert_eq!(t.parameters[0].started, now);
    observe(
        &mut t,
        &after,
        &after,
        10,
        10,
        &snapshot,
        now + Duration::from_millis(220),
    );
    assert_eq!(t.parameters[0].started, now);
    assert!(t.tick(now + Duration::from_millis(359)));
    assert_eq!(t.parameters.len(), 1);
    assert!(t.tick(now + Duration::from_millis(360)));
    assert!(t.parameters.is_empty());
    assert!(!t.tick(now + Duration::from_millis(420)));
    observe(
        &mut t,
        &before,
        &after,
        9,
        10,
        &snapshot,
        now + Duration::from_secs(30),
    );
    assert!(t.parameters.is_empty());
}
#[test]
fn gap_human_revision_bad_identity_equal_values_and_wrong_effective_values_fail_closed() {
    let now = Instant::now();
    let before = patch(None);
    let after = patch(Some(3.0));
    for bad in [
        "missing", "plan", "step", "before", "revision", "current", "old", "new", "kind", "equal",
    ] {
        let mut snapshot = receipt(9, 10, 0.0, 3.0);
        match bad {
            "missing" => snapshot["last_commit"] = Value::Null,
            "plan" => snapshot["last_commit"]["plan_id"] = json!(8),
            "step" => snapshot["last_commit"]["step_index"] = json!(1),
            "before" => snapshot["last_commit"]["before_revision"] = json!(8),
            "revision" => snapshot["last_commit"]["revision"] = json!(11),
            "current" => snapshot["current_revision"] = json!(11),
            "old" => snapshot["last_commit"]["parameter_changes"][0]["old_effective"] = json!(1),
            "new" => snapshot["last_commit"]["parameter_changes"][0]["new_effective"] = json!(4),
            "kind" => snapshot["last_commit"]["parameter_changes"][0]["kind"] = json!("gain"),
            _ => snapshot["last_commit"]["parameter_changes"][0]["new_effective"] = json!(0),
        }
        let mut t = Tracker::default();
        observe(&mut t, &before, &after, 9, 10, &snapshot, now);
        assert!(t.parameters.is_empty(), "{bad}");
    }
    for (previous, revision) in [(8, 10), (10, 11)] {
        let mut t = Tracker::default();
        observe(
            &mut t,
            &before,
            &after,
            9,
            10,
            &receipt(9, 10, 0.0, 3.0),
            now,
        );
        observe(
            &mut t,
            &before,
            &after,
            previous,
            revision,
            &receipt(9, 10, 0.0, 3.0),
            now,
        );
        assert!(t.parameters.is_empty());
        assert!(t.focus.is_empty());
    }
}
#[test]
fn reduced_off_and_new_plan_never_replay_and_pause_cancel_finish_committed_cues() {
    let now = Instant::now();
    let before = patch(None);
    let after = patch(Some(3.0));
    let mut snapshot = receipt(9, 10, 0.0, 3.0);
    let mut t = Tracker::default();
    t.set_mode(MotionMode::Reduced);
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.tick(now + Duration::from_millis(299)));
    assert!(!t.tick(now + Duration::from_millis(300)));
    t.set_mode(MotionMode::Off);
    t.set_mode(MotionMode::Full);
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.parameters.is_empty());
    snapshot["plan_id"] = json!(8);
    snapshot["last_commit"] = Value::Null;
    observe(&mut t, &after, &after, 10, 10, &snapshot, now);
    assert!(t.parameters.is_empty());
    snapshot = receipt(10, 11, 3.0, 4.0);
    snapshot["plan_id"] = json!(8);
    snapshot["last_commit"]["plan_id"] = json!(8);
    let next = patch(Some(4.0));
    observe(&mut t, &after, &next, 10, 11, &snapshot, now);
    assert_eq!(t.parameters.len(), 1);
    for status in ["paused", "cancelled", "completed"] {
        snapshot["status"] = json!(status);
        observe(
            &mut t,
            &next,
            &next,
            11,
            11,
            &snapshot,
            now + Duration::from_millis(100),
        );
        assert_eq!(t.parameters[0].started, now);
    }
    t.interrupt_parameter("dc", "value");
    assert!(t.parameters.is_empty());
    t.interrupt_module("dc");
    assert!(t.focus.is_empty());
}
#[test]
fn hidden_parameters_and_oversized_receipts_are_rejected() {
    let now = Instant::now();
    let before: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"dc","kind":"sh_asr"}]})).unwrap();
    let mut after = before.clone();
    after.nodes[0].params.insert("value1".into(), 3.0);
    let mut snapshot = receipt(9, 10, 0.0, 3.0);
    let p = &mut snapshot["last_commit"]["parameter_changes"][0];
    p["kind"] = json!("sh_asr");
    p["parameter"] = json!("value1");
    let mut t = Tracker::default();
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.parameters.is_empty());
    snapshot["last_commit"]["new_modules"] = json!((0..129)
        .map(|i| json!({"id":i.to_string(),"kind":"constant"}))
        .collect::<Vec<_>>());
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.modules.is_empty());
}
#[test]
fn recreated_module_is_arrival_but_preexisting_rerouted_cable_is_never_new() {
    let now = Instant::now();
    let before = patch(Some(2.0));
    let after = patch(None);
    let mut snapshot = receipt(9, 10, 2.0, 0.0);
    snapshot["last_commit"]["parameter_changes"] = json!([]);
    snapshot["last_commit"]["new_modules"] = json!([{"id":"dc","kind":"constant"}]);
    let mut t = Tracker::default();
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert_eq!(t.modules.len(), 1);
    assert!(t.parameters.is_empty());
    let mut before = after.clone();
    let mut after = after;
    let cable: openpatch_core::Cable = serde_json::from_value(
        json!({"id":"same","from":"dc","output":"out","to":"dc","input":"pitch"}),
    )
    .unwrap();
    before.cables.push(cable.clone());
    after.cables.push(cable);
    snapshot["last_commit"]["new_modules"] = json!([]);
    snapshot["last_commit"]["new_cables"] =
        json!([{"id":"same","from":"dc","output":"out","to":"dc","input":"pitch"}]);
    let mut t = Tracker::default();
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert_eq!(t.cables.len(), 0);
}

#[test]
fn overlapping_receipts_never_exceed_the_total_target_budget_and_old_steps_cannot_relabel_new_revisions(
) {
    let now = Instant::now();
    let before:Patch=serde_json::from_value(json!({"version":2,"nodes":(0..128).map(|i|json!({"id":format!("dc{i}"),"kind":"constant"})).collect::<Vec<_>>()})).unwrap();
    let mut after = before.clone();
    for n in &mut after.nodes {
        n.params.insert("value".into(), 1.0);
    }
    let mut snapshot = receipt(9, 10, 0.0, 1.0);
    snapshot["last_commit"]["parameter_changes"]=json!((0..128).map(|i|json!({"id":format!("dc{i}"),"kind":"constant","parameter":"value","old_effective":0,"new_effective":1})).collect::<Vec<_>>());
    let mut t = Tracker::default();
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert_eq!(t.parameters.len(), 128);
    let mut next = after.clone();
    for n in &mut next.nodes {
        n.params.insert("value".into(), 2.0);
    }
    let mut second = snapshot.clone();
    second["completed_steps"] = json!(2);
    second["last_revision"] = json!(11);
    second["current_revision"] = json!(11);
    second["last_commit"]["step_index"] = json!(1);
    second["last_commit"]["before_revision"] = json!(10);
    second["last_commit"]["revision"] = json!(11);
    for p in second["last_commit"]["parameter_changes"]
        .as_array_mut()
        .unwrap()
    {
        p["old_effective"] = json!(1);
        p["new_effective"] = json!(2);
    }
    observe(
        &mut t,
        &after,
        &next,
        10,
        11,
        &second,
        now + Duration::from_millis(100),
    );
    assert_eq!(t.parameters.len(), 128);
    assert!(t.parameters.iter().all(|p| p.old == 1.0 && p.new == 2.0));
    assert!(t.parameters.len() + t.modules.len() + t.cables.len() <= MAX_TARGETS);
    let mut malformed = second;
    malformed["last_commit"]["revision"] = json!(12);
    malformed["last_commit"]["before_revision"] = json!(11);
    malformed["last_revision"] = json!(12);
    malformed["current_revision"] = json!(12);
    observe(
        &mut t,
        &after,
        &next,
        11,
        12,
        &malformed,
        now + Duration::from_millis(150),
    );
    assert!(
        t.parameters.is_empty(),
        "same step identity cannot be repainted as a later revision"
    );
}

#[test]
fn first_observation_of_an_old_adjacent_commit_does_not_animate_and_partial_ages_keep_only_unexpired_cues(
) {
    let now = Instant::now();
    let before = patch(None);
    let after = patch(Some(3.0));
    let mut snapshot = receipt(9, 10, 0.0, 3.0);
    snapshot["last_commit"]["age_ms"] = json!(30_000);
    let mut t = Tracker::default();
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.parameters.is_empty());
    assert!(t.focus.is_empty());
    snapshot["last_commit"]["age_ms"] = json!(0);
    observe(
        &mut t,
        &before,
        &after,
        9,
        10,
        &snapshot,
        now + Duration::from_millis(1),
    );
    assert!(
        t.parameters.is_empty(),
        "an expired consumed receipt never restarts"
    );
    for (age, parameters, focus) in [
        (180, 1, 1),
        (359, 1, 1),
        (360, 0, 1),
        (419, 0, 1),
        (420, 0, 0),
    ] {
        let mut t = Tracker::default();
        snapshot["last_commit"]["age_ms"] = json!(age);
        observe(&mut t, &before, &after, 9, 10, &snapshot, now);
        assert_eq!(t.parameters.len(), parameters, "age{age}");
        assert_eq!(t.focus.len(), focus, "age{age}");
        if let Some(p) = t.parameters.first() {
            assert_eq!(p.started, now - Duration::from_millis(age));
        }
    }
    for invalid in [Value::Null, json!(-1), json!(1.5), json!("30")] {
        let mut t = Tracker::default();
        snapshot["last_commit"]["age_ms"] = invalid;
        observe(&mut t, &before, &after, 9, 10, &snapshot, now);
        assert!(t.parameters.is_empty());
    }
    let mut t = Tracker::default();
    snapshot["last_commit"]["age_ms"] = json!(0);
    snapshot["last_commit"]["invalidate_existing_cables"] = Value::Null;
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.parameters.is_empty());
    let mut t = Tracker::default();
    t.set_mode(MotionMode::Reduced);
    snapshot["last_commit"]["invalidate_existing_cables"] = json!(false);
    snapshot["last_commit"]["age_ms"] = json!(300);
    observe(&mut t, &before, &after, 9, 10, &snapshot, now);
    assert!(t.parameters.is_empty());
    assert!(t.focus.is_empty());
}
