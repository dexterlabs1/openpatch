use super::*;
use openpatch_core::{
    RecallRandomCaptureState, RecallRandomCaptureStatus as Status, RecallRandomCapturedScale,
    RecallRandomData, RecallRandomDegree, RecallRandomScale,
};

#[derive(Default)]
struct RecallProbe {
    calls: Mutex<Vec<(String, u8, u64)>>,
    capture: Mutex<Option<RecallRandomCaptureState>>,
    next_id: Mutex<u64>,
}
impl RecallProbe {
    fn record_notes(&self) {
        let mut capture = self.capture.lock().unwrap();
        let state = capture.as_mut().unwrap();
        assert_eq!(state.status, Status::Recording);
        state.scale = recorded_scale();
    }
}
fn recorded_scale() -> RecallRandomCapturedScale {
    let mut scale = RecallRandomCapturedScale {
        base_interval: 1.25,
        num_degrees: 3,
        ..RecallRandomCapturedScale::default()
    };
    scale.degrees[..3].copy_from_slice(&[
        RecallRandomDegree {
            voltage: -0.008,
            weight: 255,
        },
        RecallRandomDegree {
            voltage: 0.2,
            weight: 17,
        },
        RecallRandomDegree {
            voltage: 0.777,
            weight: 193,
        },
    ]);
    scale
}
impl LiveModuleEvents for RecallProbe {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("unused".into())
    }
    fn snapshot(&self) -> Value {
        json!({})
    }
    fn recall_random_capture(
        &self,
        id: &str,
        action: &str,
        slot: u8,
        revision: u64,
    ) -> Result<Value, String> {
        assert_eq!(id, "r");
        self.calls
            .lock()
            .unwrap()
            .push((action.into(), slot, revision));
        let mut capture = self.capture.lock().unwrap();
        match action {
            "begin" => {
                if capture.is_some_and(|s| s.status == Status::Recording) {
                    return Err("busy".into());
                }
                let mut next = self.next_id.lock().unwrap();
                *next += 1;
                *capture = Some(RecallRandomCaptureState {
                    capture_id: *next,
                    revision,
                    slot,
                    status: Status::Recording,
                    scale: RecallRandomCapturedScale::default(),
                });
            }
            "stop" => {
                let state = capture.as_mut().ok_or("not recording")?;
                if state.status != Status::Recording {
                    return Err("not recording".into());
                }
                state.status = if state.scale.num_degrees >= 2 {
                    Status::Complete
                } else {
                    Status::Empty
                };
            }
            "cancel" => {
                let state = capture.as_mut().ok_or("no capture")?;
                if state.status == Status::Cancelled {
                    return Err("already cancelled".into());
                }
                state.status = Status::Cancelled;
                state.scale = RecallRandomCapturedScale::default();
            }
            "inspect" => {}
            _ => return Err("invalid action".into()),
        }
        Ok(json!({"id":id,"instance":91,"revision":revision,"capture":*capture}))
    }
}
fn recall_session(workspace: &Path) -> Session {
    let mut s = Session::new(workspace).unwrap();
    s.execute("replace_patch",json!({"patch":{"version":2,"sample_rate":48000,"nodes":[{"id":"r","kind":"recall_random"}]}})).unwrap();
    s
}
fn recall_setup() -> (Session, Arc<RecallProbe>) {
    let mut s = recall_session(&std::env::temp_dir());
    let probe = Arc::new(RecallProbe::default());
    s.set_live_module_events(Some(probe.clone())).unwrap();
    (s, probe)
}
fn capture(s: &mut Session, action: &str) -> Result<Value, String> {
    s.execute(
        "capture_recall_random_scale",
        json!({"id":"r","action":action,"slot":2,"expected_revision":s.revision()}),
    )
}
fn weighted_content() -> openpatch_core::ModuleContent {
    let mut data = RecallRandomData::default();
    data.active_slot = 2;
    data.scales[2] = RecallRandomScale {
        base_interval: 1.25,
        degrees: vec![
            RecallRandomDegree {
                voltage: -0.008,
                weight: 255,
            },
            RecallRandomDegree {
                voltage: 0.2,
                weight: 17,
            },
            RecallRandomDegree {
                voltage: 0.777,
                weight: 193,
            },
        ],
    };
    openpatch_core::ModuleContent::RecallRandom(data.into())
}
#[test]
fn recall_random_weighted_content_history_presets_initialize_and_disk_reopen_share_exact_data() {
    let workspace = tempfile::tempdir().unwrap();
    let mut s = recall_session(workspace.path());
    let before = s.snapshot();
    let content = weighted_content();
    s.execute(
        "set_module_content",
        json!({"id":"r","content":content,"expected_revision":s.revision()}),
    )
    .unwrap();
    let authored = s.snapshot();
    assert_eq!(
        authored["patch"]["nodes"][0]["content"],
        serde_json::to_value(&content).unwrap()
    );
    assert_eq!(
        s.execute("undo", json!({})).unwrap()["patch"],
        before["patch"]
    );
    assert_eq!(
        s.execute("redo", json!({})).unwrap()["patch"],
        authored["patch"]
    );
    let preset = s
        .execute("capture_module_preset", json!({"id":"r"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["content"], authored["patch"]["nodes"][0]["content"]);
    s.execute("initialize_modules", json!({"ids":["r"]}))
        .unwrap();
    assert_eq!(
        s.snapshot()["patch"]["nodes"][0]["content"],
        preset["content"]
    );
    s.execute("set_module_content",json!({"id":"r","content":openpatch_core::ModuleContent::RecallRandom(RecallRandomData::default().into())})).unwrap();
    s.execute("apply_module_preset", json!({"id":"r","preset":preset}))
        .unwrap();
    assert_eq!(
        s.snapshot()["patch"]["nodes"][0]["content"],
        authored["patch"]["nodes"][0]["content"]
    );
    s.execute("save_patch", json!({"filename":"weighted.json"}))
        .unwrap();
    let mut reopened = Session::new(workspace.path()).unwrap();
    reopened
        .execute("load_patch", json!({"filename":"weighted.json"}))
        .unwrap();
    assert_eq!(reopened.snapshot()["patch"], s.snapshot()["patch"]);
}
#[test]
fn recall_random_invalid_or_stale_content_and_failed_batches_are_atomic() {
    let (mut s, _) = recall_setup();
    let start = s.revision();
    s.execute(
        "set_module_content",
        json!({"id":"r","content":weighted_content()}),
    )
    .unwrap();
    let before = s.snapshot();
    let mut bad_interval = serde_json::to_value(weighted_content()).unwrap();
    bad_interval["scales"][2]["base_interval"] = json!(0);
    let mut bad_weight = serde_json::to_value(weighted_content()).unwrap();
    bad_weight["scales"][2]["degrees"][0]["weight"] = json!(254);
    let mut bad_order = serde_json::to_value(weighted_content()).unwrap();
    bad_order["scales"][2]["degrees"][1]["voltage"] = json!(-0.5);
    let mut too_many = serde_json::to_value(weighted_content()).unwrap();
    too_many["scales"][2]["degrees"] = json!((0..17)
        .map(|i| json!({"voltage":i as f32/20.,"weight":255}))
        .collect::<Vec<_>>());
    for content in [
        json!({"type":"recall_random","scales":[],"active_slot":0}),
        bad_interval,
        bad_weight,
        bad_order,
        too_many,
    ] {
        assert!(s
            .execute("set_module_content", json!({"id":"r","content":content}))
            .is_err());
        assert_eq!(s.snapshot(), before);
    }
    assert!(s
        .execute(
            "set_module_content",
            json!({"id":"r","content":weighted_content(),"expected_revision":start})
        )
        .is_err());
    assert!(s.execute("apply_edits",json!({"edits":[{"tool":"set_parameter","arguments":{"id":"r","parameter":"t_rate","value":12}},{"tool":"set_module_content","arguments":{"id":"r","content":{"type":"recall_random","scales":[],"active_slot":0}}}]})).is_err());
    assert_eq!(s.snapshot(), before);
}
#[test]
fn recall_random_capture_commands_are_transient_and_only_completed_results_commit_one_history_edit()
{
    let (mut s, probe) = recall_setup();
    let before = s.snapshot();
    let history = s.history();
    let revision = s.revision();
    assert!(capture(&mut s, "commit").is_err());
    capture(&mut s, "begin").unwrap();
    assert!(capture(&mut s, "begin").is_err());
    assert_eq!(
        capture(&mut s, "inspect").unwrap()["capture"]["status"],
        "recording"
    );
    assert!(capture(&mut s, "commit").is_err());
    probe.record_notes();
    capture(&mut s, "stop").unwrap();
    assert_eq!(
        capture(&mut s, "inspect").unwrap()["capture"]["status"],
        "complete"
    );
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.history(), history);
    let receipt = capture(&mut s, "commit").unwrap();
    assert_eq!(receipt["capture_committed"]["slot"], 2);
    assert_eq!(s.revision(), revision + 1);
    let after = s.snapshot();
    assert_eq!(
        after["history"]["undo_steps"].as_u64().unwrap(),
        before["history"]["undo_steps"].as_u64().unwrap() + 1
    );
    assert_eq!(
        after["patch"]["nodes"][0]["content"]["scales"][2],
        serde_json::to_value(recorded_scale().to_scale()).unwrap()
    );
    assert!(capture(&mut s, "commit").is_err());
    assert_eq!(s.snapshot(), after);
    assert_eq!(
        s.execute("undo", json!({})).unwrap()["patch"],
        before["patch"]
    );
    assert_eq!(
        s.execute("redo", json!({})).unwrap()["patch"],
        after["patch"]
    );
}
#[test]
fn recall_random_cancel_empty_and_invalid_backend_results_never_author_scales() {
    let (mut s, probe) = recall_setup();
    let before = s.snapshot();
    capture(&mut s, "begin").unwrap();
    capture(&mut s, "stop").unwrap();
    assert_eq!(
        capture(&mut s, "inspect").unwrap()["capture"]["status"],
        "empty"
    );
    assert!(capture(&mut s, "commit").is_err());
    capture(&mut s, "begin").unwrap();
    probe.record_notes();
    capture(&mut s, "stop").unwrap();
    capture(&mut s, "cancel").unwrap();
    assert_eq!(
        capture(&mut s, "inspect").unwrap()["capture"]["status"],
        "cancelled"
    );
    assert!(capture(&mut s, "commit").is_err());
    capture(&mut s, "begin").unwrap();
    probe.record_notes();
    capture(&mut s, "stop").unwrap();
    probe
        .capture
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .scale
        .degrees[0]
        .weight = 254;
    assert!(capture(&mut s, "commit").is_err());
    assert_eq!(s.snapshot(), before);
}
#[test]
fn recall_random_authored_edit_invalidates_capture_start_revision_even_after_content_is_undone() {
    let (mut s, probe) = recall_setup();
    capture(&mut s, "begin").unwrap();
    let start = s.revision();
    probe.record_notes();
    s.execute(
        "set_parameter",
        json!({"id":"r","parameter":"t_rate","value":24}),
    )
    .unwrap();
    capture(&mut s, "stop").unwrap();
    let edited = s.snapshot();
    assert!(capture(&mut s, "commit").is_err());
    assert_eq!(s.snapshot(), edited);
    s.execute("undo", json!({})).unwrap();
    let undone = s.snapshot();
    assert!(s.revision() > start);
    assert!(capture(&mut s, "commit").is_err());
    assert_eq!(s.snapshot(), undone);
}
#[test]
fn recall_random_capture_validation_and_transaction_authority_reject_without_backend_calls() {
    let (mut s, probe) = recall_setup();
    let before = s.snapshot();
    for args in [
        json!({"id":"r","action":"begin","slot":6}),
        json!({"id":"r","action":"begin","slot":-1}),
        json!({"id":"r","action":"unknown"}),
        json!({"id":"r","action":"begin","expected_revision":0}),
        json!({"id":"missing","action":"begin"}),
        json!({"id":"r","action":"begin","surprise":1}),
    ] {
        assert!(s.execute("capture_recall_random_scale", args).is_err());
    }
    assert!(probe.calls.lock().unwrap().is_empty());
    assert!(s.execute("apply_edits",json!({"edits":[{"tool":"capture_recall_random_scale","arguments":{"id":"r","action":"begin"}}]})).is_err());
    assert!(probe.calls.lock().unwrap().is_empty());
    assert_eq!(s.snapshot(), before);
    s.set_live_module_events(None).unwrap();
    assert!(capture(&mut s, "begin").is_err());
}

#[test]
fn recall_random_identical_scale_commit_consumes_receipt_even_when_shared_content_edit_is_a_noop() {
    let (mut session, probe) = recall_setup();
    let mut data = RecallRandomData::default();
    data.scales[2] = recorded_scale().to_scale();
    session
        .execute(
            "set_module_content",
            json!({"id":"r","content":openpatch_core::ModuleContent::RecallRandom(data.into())}),
        )
        .unwrap();
    capture(&mut session, "begin").unwrap();
    probe.record_notes();
    capture(&mut session, "stop").unwrap();
    let before = session.snapshot();
    capture(&mut session, "commit").unwrap();
    assert_eq!(
        session.snapshot(),
        before,
        "identical scale needs no duplicate history edit"
    );
    assert!(
        capture(&mut session, "commit").is_err(),
        "a no-op authored edit must still consume its capture receipt"
    );
    assert_eq!(session.snapshot(), before);
}

#[test]
fn recall_random_noop_commit_distinguishes_backend_replacement_from_same_backend_reattachment() {
    let (mut session, first) = recall_setup();
    let mut data = RecallRandomData::default();
    data.scales[2] = recorded_scale().to_scale();
    session
        .execute(
            "set_module_content",
            json!({"id":"r","content":openpatch_core::ModuleContent::RecallRandom(data.into())}),
        )
        .unwrap();
    let revision = session.revision();
    capture(&mut session, "begin").unwrap();
    first.record_notes();
    capture(&mut session, "stop").unwrap();
    capture(&mut session, "commit").unwrap();
    let unchanged = session.snapshot();
    session.set_live_module_events(Some(first.clone())).unwrap();
    assert!(
        capture(&mut session, "commit").is_err(),
        "reattaching the same backend cannot reopen its consumed receipt"
    );
    session.set_live_module_events(None).unwrap();
    let replacement = Arc::new(RecallProbe::default());
    session
        .set_live_module_events(Some(replacement.clone()))
        .unwrap();
    capture(&mut session, "begin").unwrap();
    replacement.record_notes();
    capture(&mut session, "stop").unwrap();
    let fresh = capture(&mut session, "inspect").unwrap();
    assert_eq!(fresh["capture"]["capture_id"], 1);
    assert_eq!(fresh["capture"]["revision"], revision);
    assert_eq!(
        fresh["instance"], 91,
        "mock deliberately reuses its per-backend instance token"
    );
    capture(&mut session, "commit").unwrap();
    assert_eq!(session.snapshot(), unchanged);
    assert!(capture(&mut session, "commit").is_err());
}
