//! Receipts exercise the real worker/transaction boundary with controlled due
//! times. No UI, audio backend, wall-clock sized construction intervals or files.
use super::tests::{advance_one, controlled_steps, shared, snapshot, until};
use super::*;

fn step(label: &str, edits: Value) -> Value {
    json!({"label":label,"edits":edits})
}

#[test]
fn annotation_construction_content_commits_with_mixed_edits_and_history() {
    let session = shared();
    setup(
        &session,
        json!([
            {"tool":"add_module","arguments":{"id":"notes","kind":"notes"}},
            {"tool":"add_module","arguments":{"id":"blank","kind":"blank"}}
        ]),
    );
    let before = session.lock().unwrap().snapshot()["patch"].clone();
    let session = controlled_steps(
        session,
        vec![step(
            "Annotate",
            json!([
                {"tool":"set_module_content","arguments":{"id":"notes","content":{"type":"notes","text":"built\n日本語"}}},
                {"tool":"set_module_content","arguments":{"id":"blank","content":{"type":"blank","width_hp":24}}},
                {"tool":"add_module","arguments":{"id":"tone","kind":"constant","params":{"value":2}}}
            ]),
        )],
    );
    assert_eq!(advance_one(&session)["status"], "completed");
    let mut live = session.lock().unwrap();
    assert_eq!(
        live.patch.nodes[0].effective_notes_text(),
        Some("built\n日本語")
    );
    assert_eq!(live.patch.nodes[1].effective_blank_width_hp(), Some(24));
    assert_eq!(live.revision, 2);
    live.execute("undo", json!({})).unwrap();
    assert_eq!(live.snapshot()["patch"], before);
}

fn without_age(mut receipt: Value) -> Value {
    if let Some(receipt) = receipt.as_object_mut() {
        assert!(receipt.remove("age_ms").unwrap().as_u64().is_some());
    }
    receipt
}

fn setup(session: &SharedSession, edits: Value) {
    session
        .lock()
        .unwrap()
        .execute("apply_edits", json!({"edits":edits}))
        .unwrap();
}

#[test]
fn successful_receipt_has_exact_identity_graph_routes_and_effective_defaults() {
    let session = shared();
    setup(
        &session,
        json!([
            {"tool":"add_module","arguments":{"id":"z","kind":"constant"}},
            {"tool":"add_module","arguments":{"id":"a","kind":"gain"}}
        ]),
    );
    let session = controlled_steps(
        session,
        vec![step(
            "Actual committed action",
            json!([
                {"tool":"set_parameter","arguments":{"id":"z","parameter":"value","value":3}},
                {"tool":"set_parameter","arguments":{"id":"a","parameter":"gain","value":2}},
                {"tool":"add_module","arguments":{"id":"new","kind":"constant","params":{"value":8}}},
                {"tool":"set_parameter","arguments":{"id":"new","parameter":"value","value":9}},
                {"tool":"connect","arguments":{"cable_id":"wire-b","from":"z","to":"a","input":"signal"}},
                {"tool":"connect","arguments":{"cable_id":"wire-a","from":"z","to":"a","input":"signal","mode":"stack"}}
            ]),
        )],
    );
    assert!(snapshot(&session)["last_commit"].is_null());
    let state = advance_one(&session);
    assert_eq!(state["status"], "completed");
    assert_eq!(
        without_age(state["last_commit"].clone()),
        json!({
            "plan_id":1,"step_index":0,"before_revision":1,"revision":2,"label":"Actual committed action",
            "invalidate_existing_cables":false,
            "new_modules":[{"id":"new","kind":"constant"}],
            "new_cables":[
                {"id":"wire-b","from":"z","output":"out","to":"a","input":"signal"},
                {"id":"wire-a","from":"z","output":"out","to":"a","input":"signal"}
            ],
            "parameter_changes":[
                {"id":"a","kind":"gain","parameter":"gain","old_effective":1.0,"new_effective":2.0},
                {"id":"z","kind":"constant","parameter":"value","old_effective":0.0,"new_effective":3.0}
            ]
        })
    );
    let live = session.lock().unwrap();
    assert_eq!(live.revision, 2);
    assert_eq!(live.undo.len(), 2);
    assert_eq!(live.patch.cables.len(), 2);
    assert_eq!(live.patch.nodes[2].params["value"], 9.0);
}

#[test]
fn equal_effective_values_and_repeated_writes_collapse_to_initial_and_final() {
    let session = shared();
    setup(
        &session,
        json!([
            {"tool":"add_module","arguments":{"id":"equal","kind":"constant"}},
            {"tool":"add_module","arguments":{"id":"changed","kind":"constant","params":{"value":2}}}
        ]),
    );
    let session = controlled_steps(
        session,
        vec![
            step(
                "Collapse",
                json!([
                    {"tool":"set_parameter","arguments":{"id":"equal","parameter":"value","value":5}},
                    {"tool":"set_parameter","arguments":{"id":"equal","parameter":"value","value":0}},
                    {"tool":"set_parameter","arguments":{"id":"changed","parameter":"value","value":4}},
                    {"tool":"set_parameter","arguments":{"id":"changed","parameter":"value","value":7}}
                ]),
            ),
            step(
                "Equal explicit/default",
                json!([
                    {"tool":"set_parameter","arguments":{"id":"changed","parameter":"value","value":7}},
                    {"tool":"set_parameter","arguments":{"id":"equal","parameter":"value","value":0}}
                ]),
            ),
        ],
    );
    let first = advance_one(&session);
    assert_eq!(
        first["last_commit"]["parameter_changes"],
        json!([
            {"id":"changed","kind":"constant","parameter":"value","old_effective":2.0,"new_effective":7.0}
        ])
    );
    let second = advance_one(&session);
    assert_eq!(second["last_commit"]["step_index"], 1);
    assert_eq!(second["last_commit"]["before_revision"], 2);
    assert_eq!(second["last_commit"]["revision"], 3);
    assert_eq!(second["last_commit"]["parameter_changes"], json!([]));
}

#[test]
fn hidden_complete_saved_state_is_excluded_and_visible_keys_follow_catalog_order() {
    let session = shared();
    setup(
        &session,
        json!([{"tool":"add_module","arguments":{"id":"shift","kind":"sh_asr"}}]),
    );
    let params: BTreeMap<_, _> = (1..=8)
        .map(|index| (format!("value{index}"), json!(index)))
        .chain([
            ("rng_hi".into(), json!(0)),
            ("rng_lo".into(), json!(17)),
            ("random".into(), json!(1)),
            ("range".into(), json!(2)),
            ("seed".into(), json!(7)),
        ])
        .collect();
    let session = controlled_steps(
        session,
        vec![
            step(
                "Restore snapshot",
                json!([
                    {"tool":"remove_module","arguments":{"id":"shift"}},
                    {"tool":"add_module","arguments":{"id":"shift","kind":"sh_asr","params":params}}
                ]),
            ),
            step(
                "Visible initialization",
                json!([
                    {"tool":"initialize_modules","arguments":{"ids":["shift"]}}
                ]),
            ),
        ],
    );
    let first = advance_one(&session);
    assert_eq!(
        first["last_commit"]["new_modules"],
        json!([{"id":"shift","kind":"sh_asr"}])
    );
    assert_eq!(first["last_commit"]["parameter_changes"], json!([]));
    let second = advance_one(&session);
    let changes = second["last_commit"]["parameter_changes"]
        .as_array()
        .unwrap();
    assert_eq!(
        changes
            .iter()
            .map(|change| change["parameter"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["random", "range", "seed"]
    );
    assert_eq!(session.lock().unwrap().patch.nodes[0].params["value1"], 0.0);
}

#[test]
fn reused_module_id_is_an_arrival_and_preexisting_cable_ids_never_are() {
    let session = shared();
    setup(
        &session,
        json!([
            {"tool":"add_module","arguments":{"id":"source","kind":"constant","params":{"value":1}}},
            {"tool":"add_module","arguments":{"id":"gain","kind":"gain"}},
            {"tool":"connect","arguments":{"cable_id":"reused","from":"source","to":"gain","input":"signal"}},
            {"tool":"connect","arguments":{"cable_id":"moved","from":"source","to":"gain","input":"cv"}}
        ]),
    );
    let session = controlled_steps(
        session,
        vec![
            step(
                "Move existing identity",
                json!([
                    {"tool":"move_cables","arguments":{"cable_ids":["moved"],"end":"input","node":"gain","port":"signal","duplicate_mode":"stack"}}
                ]),
            ),
            step(
                "Replace same identity",
                json!([
                    {"tool":"disconnect","arguments":{"cable_id":"reused"}},
                    {"tool":"connect","arguments":{"cable_id":"reused","from":"source","to":"gain","input":"signal","mode":"stack"}},
                    {"tool":"remove_module","arguments":{"id":"source"}},
                    {"tool":"add_module","arguments":{"id":"source","kind":"constant","params":{"value":8}}},
                    {"tool":"connect","arguments":{"cable_id":"reused","from":"source","to":"gain","input":"signal"}},
                    {"tool":"connect","arguments":{"cable_id":"fresh","from":"source","to":"gain","input":"cv"}}
                ]),
            ),
        ],
    );
    let moved = advance_one(&session)["last_commit"].clone();
    assert_eq!(moved["invalidate_existing_cables"], true);
    assert_eq!(moved["new_cables"], json!([]));
    let receipt = advance_one(&session)["last_commit"].clone();
    assert_eq!(receipt["invalidate_existing_cables"], true);
    assert_eq!(
        receipt["new_modules"],
        json!([{"id":"source","kind":"constant"}])
    );
    assert_eq!(receipt["parameter_changes"], json!([]));
    assert_eq!(
        receipt["new_cables"],
        json!([
            {"id":"fresh","from":"source","output":"out","to":"gain","input":"cv"}
        ])
    );
}

#[test]
fn target_budget_is_shared_by_module_cable_and_parameter_categories() {
    let session = shared();
    let mut edits: Vec<Value> = (0..127)
        .map(|index| {
            json!({"tool":"add_module","arguments":{
        "id":format!("source{index:03}"),"kind":"constant"}})
        })
        .collect();
    edits.push(json!({"tool":"add_module","arguments":{"id":"gain","kind":"gain"}}));
    setup(&session, json!(edits));
    let copies: Vec<Value> = (0..127)
        .map(|index| json!({"id":format!("source{index:03}"),"new_id":format!("copy{index:03}")}))
        .collect();
    let session = controlled_steps(
        session,
        vec![step(
            "Bound all categories",
            json!([
                {"tool":"duplicate_modules","arguments":{"modules":copies}},
                {"tool":"connect","arguments":{"cable_id":"first","from":"copy000","to":"gain","input":"signal"}},
                {"tool":"connect","arguments":{"cable_id":"overflow","from":"copy001","to":"gain","input":"cv"}},
                {"tool":"set_parameter","arguments":{"id":"gain","parameter":"gain","value":3}}
            ]),
        )],
    );
    let receipt = advance_one(&session)["last_commit"].clone();
    assert_eq!(receipt["new_modules"].as_array().unwrap().len(), 127);
    assert_eq!(receipt["new_cables"].as_array().unwrap().len(), 1);
    assert_eq!(receipt["new_cables"][0]["id"], "first");
    assert_eq!(receipt["parameter_changes"], json!([]));
    let live = session.lock().unwrap();
    assert_eq!(live.patch.nodes.len(), 255);
    assert_eq!(live.patch.cables.len(), 2);
    assert_eq!(live.patch.nodes[127].params["gain"], 3.0);
}

#[test]
fn overflowing_parameter_changes_are_ordered_and_graph_is_complete() {
    let session = shared();
    setup(
        &session,
        json!((0..130)
            .rev()
            .map(|index| json!({"tool":"add_module","arguments":{
        "id":format!("node{index:03}"),"kind":"constant","params":{"value":1}}}))
            .collect::<Vec<_>>()),
    );
    let ids: Vec<_> = (0..130)
        .rev()
        .map(|index| format!("node{index:03}"))
        .collect();
    let session = controlled_steps(
        session,
        vec![step(
            "Initialize many",
            json!([
                {"tool":"initialize_modules","arguments":{"ids":ids}}
            ]),
        )],
    );
    let receipt = advance_one(&session)["last_commit"].clone();
    let changes = receipt["parameter_changes"].as_array().unwrap();
    assert_eq!(changes.len(), 128);
    assert_eq!(changes[0]["id"], "node000");
    assert_eq!(changes[127]["id"], "node127");
    assert!(session.lock().unwrap().patch.nodes.iter().all(|node| node
        .params
        .get("value")
        .copied()
        .unwrap_or(0.0)
        == 0.0));
}

#[test]
fn overflowing_module_arrivals_keep_final_graph_order_and_full_graph() {
    let session = shared();
    setup(
        &session,
        json!([{"tool":"add_module","arguments":{"id":"node000","kind":"constant"}}]),
    );
    let mut edits = Vec::new();
    for power in 0..8 {
        let count = 1 << power;
        let copies: Vec<_> = (0..count)
            .map(|index| {
                json!({
                    "id":format!("node{index:03}"),"new_id":format!("node{:03}", index + count)
                })
            })
            .collect();
        edits.push(json!({"tool":"duplicate_modules","arguments":{"modules":copies}}));
    }
    let session = controlled_steps(
        session,
        vec![step("Grow within one real step", json!(edits))],
    );
    let receipt = advance_one(&session)["last_commit"].clone();
    let modules = receipt["new_modules"].as_array().unwrap();
    assert_eq!(modules.len(), 128);
    assert_eq!(modules[0]["id"], "node001");
    assert_eq!(modules[127]["id"], "node128");
    assert_eq!(receipt["parameter_changes"], json!([]));
    assert_eq!(session.lock().unwrap().patch.nodes.len(), 256);
}

#[test]
fn receipt_survives_pause_cancel_completion_and_new_plan_clears_it() {
    let session = controlled_steps(
        shared(),
        vec![
            step(
                "First",
                json!([
                    {"tool":"add_module","arguments":{"id":"source","kind":"constant"}}
                ]),
            ),
            step(
                "Second",
                json!([
                    {"tool":"set_parameter","arguments":{"id":"source","parameter":"value","value":1}}
                ]),
            ),
        ],
    );
    let first = without_age(advance_one(&session)["last_commit"].clone());
    for command in ["pause_construction", "cancel_construction"] {
        let state = execute_shared(&session, command, json!({"plan_id":1})).unwrap();
        assert_eq!(without_age(state["last_commit"].clone()), first);
    }
    let next = execute_shared(
        &session,
        "start_construction",
        json!({
            "expected_revision":1,"interval_ms":2000,"steps":[step("New plan", json!([
                {"tool":"set_parameter","arguments":{"id":"source","parameter":"value","value":3}}
            ]))]
        }),
    )
    .unwrap();
    assert_eq!(next["plan_id"], 2);
    assert!(next["last_commit"].is_null());
    // Use the same controlled-deadline mechanism for the admitted real plan.
    let done = advance_one(&session);
    assert_eq!(done["status"], "completed");
    let last = without_age(done["last_commit"].clone());
    assert_eq!(last["plan_id"], 2);
    assert_eq!(without_age(snapshot(&session)["last_commit"].clone()), last);
}

#[test]
fn failed_step_never_publishes_a_receipt_or_replaces_a_successful_one() {
    // Deliberately install a bad step in the private worker fixture, after the
    // admission/preflight boundary, to exercise runtime transaction failure.
    for successful_prefix in [false, true] {
        let mut steps = Vec::new();
        if successful_prefix {
            steps.push(step(
                "Successful",
                json!([
                    {"tool":"add_module","arguments":{"id":"source","kind":"constant"}}
                ]),
            ));
        }
        steps.push(step(
            "Fail atomically",
            json!([
                {"tool":"add_module","arguments":{"id":"uncommitted","kind":"constant"}},
                {"tool":"set_parameter","arguments":{"id":"missing","parameter":"value","value":1}}
            ]),
        ));
        let session = controlled_steps(shared(), steps);
        let previous = if successful_prefix {
            without_age(advance_one(&session)["last_commit"].clone())
        } else {
            Value::Null
        };
        {
            let mut live = session.lock().unwrap();
            let plan = live.construction.plan.as_mut().unwrap();
            plan.next_due = Instant::now();
            plan.wake.notify();
        }
        let failed = until(&session, |state| state["status"] == "failed");
        assert_eq!(without_age(failed["last_commit"].clone()), previous);
        assert_eq!(failed["completed_steps"], usize::from(successful_prefix));
        assert_eq!(failed["current_revision"], usize::from(successful_prefix));
        assert!(session
            .lock()
            .unwrap()
            .patch
            .nodes
            .iter()
            .all(|node| node.id != "uncommitted"));
    }
}

#[test]
fn same_route_cable_recreation_invalidates_existing_cues_but_only_fresh_ids_arrive() {
    let session = shared();
    setup(
        &session,
        json!([
            {"tool":"add_module","arguments":{"id":"source","kind":"constant"}},
            {"tool":"add_module","arguments":{"id":"gain","kind":"gain"}}
        ]),
    );
    let session = controlled_steps(
        session,
        vec![
            step(
                "Connect",
                json!([
                    {"tool":"connect","arguments":{"cable_id":"same","from":"source","to":"gain","input":"signal"}}
                ]),
            ),
            step(
                "Replace same route",
                json!([
                    {"tool":"disconnect","arguments":{"cable_id":"same"}},
                    {"tool":"connect","arguments":{"cable_id":"same","from":"source","to":"gain","input":"signal"}},
                    {"tool":"connect","arguments":{"cable_id":"fresh","from":"source","to":"gain","input":"cv"}}
                ]),
            ),
            step(
                "Failed retirement",
                json!([
                    {"tool":"disconnect","arguments":{"cable_id":"same"}},
                    {"tool":"set_parameter","arguments":{"id":"missing","parameter":"value","value":1}}
                ]),
            ),
        ],
    );
    let first = advance_one(&session)["last_commit"].clone();
    assert_eq!(first["invalidate_existing_cables"], false);
    assert_eq!(first["new_cables"][0]["id"], "same");
    let second = advance_one(&session)["last_commit"].clone();
    assert_eq!(second["invalidate_existing_cables"], true);
    assert_eq!(second["new_cables"].as_array().unwrap().len(), 1);
    assert_eq!(second["new_cables"][0]["id"], "fresh");
    {
        let mut live = session.lock().unwrap();
        let plan = live.construction.plan.as_mut().unwrap();
        plan.next_due = Instant::now();
        plan.wake.notify();
    }
    let failed = until(&session, |state| state["status"] == "failed");
    assert_eq!(
        without_age(failed["last_commit"].clone()),
        without_age(second)
    );
    assert_eq!(session.lock().unwrap().patch.cables.len(), 2);
}

#[test]
fn commit_age_uses_monotonic_time_and_snapshots_pause_and_cancel_do_not_refresh_it() {
    let session = controlled_steps(
        shared(),
        vec![
            step(
                "Commit now",
                json!([
                    {"tool":"add_module","arguments":{"id":"source","kind":"constant"}}
                ]),
            ),
            step(
                "Pending",
                json!([
                    {"tool":"set_parameter","arguments":{"id":"source","parameter":"value","value":1}}
                ]),
            ),
        ],
    );
    advance_one(&session);
    let committed_at = {
        let mut live = session.lock().unwrap();
        let receipt = live
            .construction
            .plan
            .as_mut()
            .unwrap()
            .last_commit
            .as_mut()
            .unwrap();
        let committed_at = receipt.committed_at;
        assert_eq!(receipt.to_json_at(committed_at)["age_ms"], 0);
        assert_eq!(
            receipt.to_json_at(committed_at + Duration::from_micros(999_999))["age_ms"],
            999
        );
        assert_eq!(
            receipt.to_json_at(committed_at + Duration::from_secs(30))["age_ms"],
            30_000
        );
        assert_eq!(
            receipt.to_json_at(committed_at - Duration::from_secs(1))["age_ms"],
            0
        );
        // Model a hidden client without a 30-second test sleep. Production uses
        // Instant::now; retained snapshots must report this aged real commit.
        receipt.committed_at = committed_at - Duration::from_secs(30);
        receipt.committed_at
    };
    let first = snapshot(&session)["last_commit"].clone();
    let mut previous_age = first["age_ms"].as_u64().unwrap();
    assert!(previous_age >= 30_000);
    for command in [
        "inspect_construction",
        "pause_construction",
        "cancel_construction",
    ] {
        let state = execute_shared(&session, command, json!({"plan_id":1})).unwrap();
        let age = state["last_commit"]["age_ms"].as_u64().unwrap();
        assert!(age >= previous_age);
        previous_age = age;
        assert_eq!(
            without_age(state["last_commit"].clone()),
            without_age(first.clone())
        );
        assert_eq!(
            session
                .lock()
                .unwrap()
                .construction
                .plan
                .as_ref()
                .unwrap()
                .last_commit
                .as_ref()
                .unwrap()
                .committed_at,
            committed_at
        );
    }
}
