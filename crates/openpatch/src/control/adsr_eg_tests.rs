use super::*;

fn envelope_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute("replace_patch", json!({"patch":{"version":2,"sample_rate":48000,
        "nodes":[{"id":"env","kind":"adsr_eg","params":{"attack":0,"decay":0,"sustain":0.25,"release":0}}]}})).unwrap();
    s
}

#[test]
fn envelope_mcp_capture_preserves_cold_start_peak_release_and_history() {
    let mut s = envelope_session();
    let before = s.snapshot();
    let args = json!({"id":"env","port":"out","seconds":0.004,"points":192,
        "module_events":[{"frame":0,"id":"env","action":"push_press"},
                         {"frame":160,"id":"env","action":"push_release"}]});
    let capture = s.execute("capture_signal", args.clone()).unwrap();
    let bins = capture["bins"].as_array().unwrap();
    assert_eq!(bins.len(), 192);
    assert!(bins[..15].iter().all(|b| b["mean"] == 0.0));
    assert!(bins[15]["mean"].as_f64().unwrap() > 0.0);
    let peak = bins
        .iter()
        .map(|b| b["max"].as_f64().unwrap())
        .fold(0.0, f64::max);
    assert!((10.0..10.1).contains(&peak));
    assert!(bins[160]["mean"].as_f64().unwrap() < bins[159]["mean"].as_f64().unwrap());
    assert!(bins[191]["mean"].as_f64().unwrap() > 0.0);
    assert_eq!(s.execute("capture_signal", args).unwrap(), capture);
    assert_eq!(s.snapshot(), before);
}

#[test]
fn envelope_same_sample_actions_are_sampled_and_invalid_actions_rejected() {
    let mut s = envelope_session();
    for actions in [vec!["push_press", "push_release"], vec!["push_release"]] {
        let events: Vec<_> = actions
            .into_iter()
            .map(|action| json!({"frame":0,"id":"env","action":action}))
            .collect();
        let capture = s
            .execute(
                "capture_signal",
                json!({"id":"env","seconds":0.004,"module_events":events}),
            )
            .unwrap();
        assert!(capture["bins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["mean"] == 0.0));
    }
    for action in ["hold_press", "reset_press", "clock"] {
        assert!(s
            .execute(
                "capture_signal",
                json!({"id":"env","seconds":0.004,
            "module_events":[{"frame":0,"id":"env","action":action}]})
            )
            .is_err());
    }
    s.execute("set_bypass", json!({"id":"env","bypassed":true}))
        .unwrap();
    assert!(s
        .execute(
            "capture_signal",
            json!({"id":"env","seconds":0.004,
        "module_events":[{"frame":0,"id":"env","action":"push_press"}]})
        )
        .is_err());
    assert!(s
        .execute(
            "capture_signal",
            json!({"id":"env","seconds":0.004,
        "module_events":[{"frame":0,"id":"env","action":"push_release"}]})
        )
        .is_ok());
}

#[test]
fn envelope_live_action_receipts_are_transient_and_guarded() {
    #[derive(Default)]
    struct Live(Mutex<Vec<ModuleAction>>);
    impl LiveModuleEvents for Live {
        fn trigger(&self, _: &str, action: ModuleAction) -> Result<Value, String> {
            self.0.lock().unwrap().push(action);
            Ok(json!({"queued":true,"applied":false}))
        }
        fn snapshot(&self) -> Value {
            json!({})
        }
    }
    let mut s = envelope_session();
    let live = Arc::new(Live::default());
    s.set_live_module_events(Some(live.clone()));
    let before = s.snapshot();
    for action in ["push_press", "push_release"] {
        let receipt = s
            .execute(
                "trigger_module",
                json!({"id":"env","action":action,"expected_revision":s.revision()}),
            )
            .unwrap();
        assert_eq!(receipt["receipt"]["queued"], true);
        assert_eq!(receipt["receipt"]["applied"], false);
    }
    assert_eq!(s.snapshot(), before);
    assert!(s
        .execute("trigger_module", json!({"id":"env","action":"hold_press"}))
        .is_err());
    assert!(s
        .execute(
            "trigger_module",
            json!({"id":"env","action":"push_press","expected_revision":0})
        )
        .is_err());
    assert_eq!(live.0.lock().unwrap().len(), 2);
}
