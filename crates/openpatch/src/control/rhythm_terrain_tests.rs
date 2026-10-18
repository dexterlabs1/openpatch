use super::*;

fn terrain_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch":{"version":2,"sample_rate":48000,
        "nodes":[{"id":"terrain","kind":"rhythm_terrain","params":{"tempo":0}}],
        "output":"terrain","output_port":"clock"}}),
    )
    .unwrap();
    s
}

#[test]
fn rhythm_terrain_offline_taps_start_clock_without_editing_saved_controls() {
    let mut s = terrain_session();
    let before = s.snapshot();
    let silent = s
        .execute(
            "capture_signal",
            json!({"id":"terrain","port":"clock","seconds":0.6}),
        )
        .unwrap();
    assert!(silent["bins"]
        .as_array()
        .unwrap()
        .iter()
        .all(|b| b["max"] == 0.0));
    let args = json!({"id":"terrain","port":"clock","seconds":0.6,"points":512,
        "module_events":[{"frame":0,"id":"terrain","action":"tap_press"},
            {"frame":6,"id":"terrain","action":"tap_release"},
            {"frame":24000,"id":"terrain","action":"tap_press"},
            {"frame":24006,"id":"terrain","action":"tap_release"},
            {"frame":27000,"id":"terrain","action":"release_tap"}]});
    let actual = s.execute("capture_signal", args.clone()).unwrap();
    assert!(actual["bins"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["max"] == 5.0));
    assert_eq!(s.execute("capture_signal", args).unwrap(), actual);
    assert_eq!(s.snapshot(), before);
}

#[test]
fn rhythm_terrain_live_actions_are_revision_fenced_transient_and_release_safe() {
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
    let mut s = terrain_session();
    let live = Arc::new(Live::default());
    s.set_live_module_events(Some(live.clone())).unwrap();
    let before = s.snapshot();
    for action in ["tap_press", "tap_release", "release_tap", "reset"] {
        let receipt = s
            .execute(
                "trigger_module",
                json!({"id":"terrain","action":action,
            "expected_revision":s.revision()}),
            )
            .unwrap();
        assert_eq!(receipt["receipt"]["queued"], true);
        assert_eq!(receipt["receipt"]["applied"], false);
    }
    assert_eq!(s.snapshot(), before);
    assert!(s
        .execute(
            "trigger_module",
            json!({"id":"terrain","action":"tap_press","expected_revision":0})
        )
        .is_err());
    assert!(s
        .execute("trigger_module", json!({"id":"terrain","action":"clock"}))
        .is_err());
    s.execute("set_bypass", json!({"id":"terrain","bypassed":true}))
        .unwrap();
    for action in ["tap_press", "release_tap", "reset"] {
        assert!(s
            .execute("trigger_module", json!({"id":"terrain","action":action}))
            .is_err());
    }
    s.execute(
        "trigger_module",
        json!({"id":"terrain","action":"tap_release"}),
    )
    .unwrap();
    assert_eq!(live.0.lock().unwrap().len(), 5);
}
