use super::*;
use openpatch_core::{rackmcp_sequence::SequenceData, FixedPatternName, RackMcpTransport};

#[derive(Default)]
struct TransportProbe {
    calls: Mutex<Vec<Vec<RackMcpTransport>>>,
    published: Mutex<Value>,
}
impl LiveModuleEvents for TransportProbe {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("unused".into())
    }
    fn supports_live_parameters(&self) -> bool {
        true
    }
    fn snapshot(&self) -> Value {
        self.published.lock().unwrap().clone()
    }
    fn rackmcp_transport(
        &self,
        id: &str,
        requests: &[RackMcpTransport],
        data: &SequenceData,
    ) -> Result<Value, String> {
        assert_eq!(id, "seq");
        assert_eq!(data.patterns[1].name, "B");
        self.calls.lock().unwrap().push(requests.to_vec());
        Ok(json!({"queued":true,"applied":false}))
    }
    fn sync_patch_with_intents(
        &self,
        _: &Patch,
        revision: u64,
        _: &BTreeMap<String, BTreeMap<String, f32>>,
        _: &[String],
    ) {
        self.published.lock().unwrap()["modules"]["seq"]["revision"] = json!(revision);
    }
}
fn setup() -> (Session, Arc<TransportProbe>) {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session.execute("add_module", json!({"id":"seq","kind":"rackmcp_sequencer",
        "rackmcp_sequence":{"data":{"patterns":[{"name":"A","length":4},{"name":"B","length":8}]}}})).unwrap();
    let probe = Arc::new(TransportProbe::default());
    *probe.published.lock().unwrap() = json!({"coherent":true,"modules":{"seq":{
        "kind":"rackmcp_sequencer","available":true,"revision":session.revision,"frame":128,
        "parameters":{"running":1},"state":{"selected_pattern":"B"},"rackmcp_sequence":{"running":true,
        "active_pattern":0,"active_pattern_name":"A","pending_pattern":1,
        "pending_pattern_name":"B","selected_pattern":1,"selected_pattern_name":"B",
        "step":2,"loop_count":0,"beat":0.5,"bar":0.125,"bpm":120,"swing":50,
        "external_clock":false,"pattern_count":2,"lane_count":0}}}});
    session.set_live_module_events(Some(probe.clone())).unwrap();
    (session, probe)
}

#[test]
fn live_transport_queues_named_pattern_and_action_atomically_without_history_or_authored_edit() {
    let (mut session, probe) = setup();
    let before = session.snapshot();
    let history = session.history();
    let result = session
        .execute(
            "rackmcp_transport",
            json!({"id":"seq","action":"play","pattern":"B","quantize":"now"}),
        )
        .unwrap();
    assert_eq!(result["receipt"]["queued"], true);
    assert_eq!(
        *probe.calls.lock().unwrap(),
        vec![vec![
            RackMcpTransport::Select {
                pattern: 1,
                now: true
            },
            RackMcpTransport::Play
        ]]
    );
    assert_eq!(session.snapshot(), before);
    assert_eq!(session.history(), history);
    session
        .execute(
            "rackmcp_transport",
            json!({"id":"seq","action":"select","pattern":"A"}),
        )
        .unwrap();
    session
        .execute(
            "rackmcp_transport",
            json!({"id":"seq","action":"seek","bar":1.25}),
        )
        .unwrap();
    assert_eq!(
        probe.calls.lock().unwrap()[1],
        [RackMcpTransport::Select {
            pattern: 0,
            now: false
        }]
    );
    assert_eq!(
        probe.calls.lock().unwrap()[2],
        [RackMcpTransport::Seek { bar: 1.25 }]
    );
}

#[test]
fn invalid_transport_is_rejected_before_queue_or_history_changes() {
    let (mut session, probe) = setup();
    let before = session.snapshot();
    for args in [
        json!({"id":"seq","action":"select"}),
        json!({"id":"seq","action":"select","pattern":"missing"}),
        json!({"id":"seq","action":"play","quantize":"now"}),
        json!({"id":"seq","action":"play","bar":0}),
        json!({"id":"seq","action":"seek"}),
        json!({"id":"seq","action":"seek","bar":1,"pattern":"B"}),
        json!({"id":"seq","action":"seek","bar":-1}),
        json!({"id":"seq","action":"seek","bar":100001}),
        json!({"id":"seq","action":"play","expected_revision":0}),
        json!({"id":"seq","action":"play","surprise":true}),
    ] {
        assert!(session.execute("rackmcp_transport", args).is_err());
    }
    assert!(probe.calls.lock().unwrap().is_empty());
    assert_eq!(session.snapshot(), before);
}

#[test]
fn status_uses_actual_publication_and_reports_unavailable_without_fallback() {
    let (mut session, probe) = setup();
    let result = session
        .execute("inspect_rackmcp_sequence", json!({"id":"seq"}))
        .unwrap();
    assert_eq!(result["available"], true);
    assert_eq!(result["rackmcp_sequence"]["selected_pattern_name"], "B");
    assert_eq!(result["rackmcp_sequence"]["active_pattern_name"], "A");
    probe.published.lock().unwrap()["modules"]["seq"]["revision"] = json!(0);
    let result = session
        .execute("inspect_rackmcp_sequence", json!({"id":"seq"}))
        .unwrap();
    assert_eq!(result["available"], false);
    assert!(result.get("rackmcp_sequence").is_none());
    session.set_live_module_events(None).unwrap();
    assert_eq!(
        session
            .execute("inspect_rackmcp_sequence", json!({"id":"seq"}))
            .unwrap()["available"],
        false
    );
    assert!(session
        .execute("rackmcp_transport", json!({"id":"seq","action":"play"}))
        .is_err());
}

#[test]
fn selection_capture_and_presets_use_live_name_without_rewinding_parameter_history() {
    let (mut session, probe) = setup();
    let captured = session.capture_project_patch(None).unwrap();
    assert_eq!(
        captured.patch.nodes[0]
            .rackmcp_sequence
            .as_ref()
            .unwrap()
            .data
            .selected_pattern
            .as_deref(),
        Some("B")
    );
    assert!(session.patch.nodes[0]
        .rackmcp_sequence
        .as_ref()
        .unwrap()
        .data
        .selected_pattern
        .is_none());
    assert_eq!(captured.patch.nodes[0].params["running"], 1.0);
    let live_preset = session
        .execute("capture_module_preset", json!({"id":"seq"}))
        .unwrap();
    assert_eq!(live_preset["preset"]["params"]["running"], 1.0);
    assert_eq!(
        live_preset["preset"]["rackmcp_sequence"]["data"]["selectedPattern"],
        "B"
    );
    let preset = crate::presets::capture_state(
        &session.patch.nodes[0],
        captured.saved_states.get("seq").copied(),
    )
    .unwrap();
    assert_eq!(preset["rackmcp_sequence"]["data"]["selectedPattern"], "B");
    session
        .execute(
            "set_parameter",
            json!({"id":"seq","parameter":"running","value":1}),
        )
        .unwrap();
    assert!(!session
        .undo
        .back()
        .unwrap()
        .restore_saved
        .contains_key("seq"));
    probe.published.lock().unwrap()["modules"]["seq"]["state"]["selected_pattern"] = json!("A");
    session.execute("undo", json!({})).unwrap();
    assert_eq!(
        session.capture_project_patch(None).unwrap().saved_states["seq"],
        SavedModuleState::RackMcpSelection(FixedPatternName::new("A").unwrap())
    );
    probe.published.lock().unwrap()["modules"]["seq"]["state"]["selected_pattern"] =
        json!("not-in-data");
    assert!(session.capture_project_patch(None).is_err());
}
