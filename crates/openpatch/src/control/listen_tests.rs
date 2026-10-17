use super::*;

#[derive(Default)]
struct ListenProbe {
    calls: Mutex<Vec<Value>>,
    published: Mutex<Value>,
}
impl LiveModuleEvents for ListenProbe {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("unused".into())
    }
    fn snapshot(&self) -> Value {
        self.published.lock().unwrap().clone()
    }
    fn start_listen_capture(
        &self,
        id: &str,
        seconds: f64,
        wav: bool,
        workspace: &Path,
    ) -> Result<Value, String> {
        assert!(workspace.is_dir());
        self.calls
            .lock()
            .unwrap()
            .push(json!({"id":id,"seconds":seconds,"wav":wav}));
        Ok(json!({"job":"listen-1","id":id,"state":"capturing","phase":"arming"}))
    }
    fn inspect_listen_capture(&self, job: &str) -> Result<Value, String> {
        Ok(json!({"job":job,"state":"failed","result":{"error":"instance removed"}}))
    }
    fn cancel_listen_capture(&self, job: &str) -> Result<Value, String> {
        Ok(json!({"job":job,"state":"capturing","cancel_requested":true}))
    }
}
fn setup() -> (Session, Arc<ListenProbe>) {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute("add_module", json!({"id":"listen","kind":"rackmcp_listen"}))
        .unwrap();
    let probe = Arc::new(ListenProbe::default());
    session.set_live_module_events(Some(probe.clone())).unwrap();
    (session, probe)
}
#[test]
fn capture_requests_choose_single_listen_validate_revision_and_preserve_document_history() {
    let (mut session, probe) = setup();
    let before = session.snapshot();
    let history = session.history();
    let result = session
        .execute("start_listen_capture", json!({"seconds":0.1}))
        .unwrap();
    assert_eq!(result["capture"]["job"], "listen-1");
    assert_eq!(
        probe.calls.lock().unwrap()[0],
        json!({"id":"listen","seconds":0.1,"wav":false})
    );
    session
        .execute(
            "start_listen_capture",
            json!({"id":"listen","seconds":30,"write_wav":true}),
        )
        .unwrap();
    session
        .execute("inspect_listen_capture", json!({"job":"listen-1"}))
        .unwrap();
    assert_eq!(
        session
            .execute("cancel_listen_capture", json!({"job":"listen-1"}))
            .unwrap()["capture"]["cancel_requested"],
        true
    );
    for args in [
        json!({"seconds":0.09}),
        json!({"seconds":30.1}),
        json!({"seconds":1,"expected_revision":0}),
        json!({"seconds":1,"id":"missing"}),
        json!({"seconds":1,"id":7}),
        json!({"seconds":1,"write_wav":"yes"}),
    ] {
        assert!(
            session
                .execute("start_listen_capture", args.clone())
                .is_err(),
            "{args}"
        );
    }
    assert_eq!(probe.calls.lock().unwrap().len(), 2);
    assert_eq!(session.snapshot(), before);
    assert_eq!(session.history(), history);
    session
        .execute("add_module", json!({"id":"second","kind":"rackmcp_listen"}))
        .unwrap();
    assert!(session
        .execute("start_listen_capture", json!({"seconds":1}))
        .unwrap_err()
        .contains("multiple"));
    session
        .execute("remove_module", json!({"id":"listen"}))
        .unwrap();
    assert_eq!(
        session
            .execute("inspect_listen_capture", json!({"job":"listen-1"}))
            .unwrap()["capture"]["state"],
        "failed"
    );
}
#[test]
fn listen_inspection_requires_coherent_current_actual_publication_without_authored_fallback() {
    let (mut session, probe) = setup();
    let unavailable = session
        .execute("inspect_listen", json!({"id":"listen"}))
        .unwrap();
    assert_eq!(unavailable["available"], false);
    assert!(unavailable.get("listen").is_none());
    *probe.published.lock().unwrap() = json!({"coherent":true,"modules":{"listen":{"kind":"rackmcp_listen","revision":session.revision,"available":true,"frame":256,"listen":{"frames_seen":256,"dropped":0,"left_connected":true,"right_connected":false,"capturing":true}}}});
    let live = session
        .execute("inspect_listen", json!({"id":"listen"}))
        .unwrap();
    assert_eq!(live["available"], true);
    assert_eq!(live["listen"]["frames_seen"], 256);
    for (path, value) in [
        ("coherent", json!(false)),
        ("revision", json!(0)),
        ("available", json!(false)),
        ("kind", json!("scope")),
        ("listen", Value::Null),
    ] {
        let saved = probe.published.lock().unwrap().clone();
        {
            let mut snapshot = probe.published.lock().unwrap();
            if path == "coherent" {
                snapshot[path] = value;
            } else {
                snapshot["modules"]["listen"][path] = value;
            }
        }
        let result = session
            .execute("inspect_listen", json!({"id":"listen"}))
            .unwrap();
        assert_eq!(result["available"], false, "{path}");
        assert!(result.get("listen").is_none());
        *probe.published.lock().unwrap() = saved;
    }
    session.set_live_module_events(None).unwrap();
    assert!(session
        .execute("start_listen_capture", json!({"seconds":1}))
        .is_err());
    assert_eq!(
        session
            .execute("inspect_listen", json!({"id":"listen"}))
            .unwrap()["available"],
        false
    );
}
