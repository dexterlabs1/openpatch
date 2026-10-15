//! Shared history boundaries for native rack layout gestures.
use super::*;

impl Session {
    /// Record one rack-layout history state without changing the graph or
    /// retiring DSP identities. The host stores geometry under the returned
    /// state_id; ordinary shared undo/redo traverses the same boundary.
    pub fn record_layout_checkpoint(&mut self, expected_revision: u64) -> Result<Value, String> {
        self.commit(
            self.patch.clone(),
            Some(expected_revision),
            None,
            &EditEffects::default(),
        )?;
        Ok(self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Live {
        calls: Mutex<Vec<Value>>,
        reads: AtomicUsize,
    }
    impl LiveModuleEvents for Live {
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            self.calls.lock().unwrap().push(json!({"trigger":true}));
            Err("unexpected action".into())
        }
        fn snapshot(&self) -> Value {
            self.reads.fetch_add(1, Ordering::Relaxed);
            // Layout checkpoints must work without published Run telemetry.
            json!({"coherent":false,"modules":{}})
        }
        fn sync_patch(&self, patch: &Patch, revision: u64) {
            self.calls
                .lock()
                .unwrap()
                .push(json!({"sync":revision,"patch":patch}));
        }
        fn retire_modules(&self, ids: &[String]) {
            self.calls.lock().unwrap().push(json!({"retire":ids}));
        }
        fn retire_cables(&self, ids: &[String]) {
            self.calls
                .lock()
                .unwrap()
                .push(json!({"retire_cables":ids}));
        }
        fn initialize_modules(&self, ids: &[String]) {
            self.calls.lock().unwrap().push(json!({"initialize":ids}));
        }
        fn load_module_running(&self, states: &BTreeMap<String, bool>) {
            self.calls
                .lock()
                .unwrap()
                .push(json!({"load_running":states}));
        }
        fn release_module_controls(&self, ids: &[String]) {
            self.calls.lock().unwrap().push(json!({"release":ids}));
        }
    }

    fn session() -> (Session, Arc<Live>) {
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        session.execute("replace_patch",json!({"patch":{"version":2,"nodes":[
            {"id":"source","kind":"constant","params":{"value":0}},
            {"id":"seq","kind":"seq3","params":{"running":1,"row1_1":3}},
            {"id":"logic","kind":"logic"}],"cables":[
            {"id":"clock","from":"source","to":"seq","input":"clock"}],"output":"seq","output_port":"cv1"}})).unwrap();
        let live = Arc::new(Live::default());
        session.set_live_module_events(Some(live.clone())).unwrap();
        live.calls.lock().unwrap().clear();
        (session, live)
    }

    #[test]
    fn checkpoint_returns_normal_receipt_with_new_state_and_only_synchronizes_graph() {
        let (mut session, live) = session();
        let before = session.snapshot();
        let receipt = session.record_layout_checkpoint(1).unwrap();
        assert_eq!(receipt, session.snapshot());
        assert_eq!(receipt["patch"], before["patch"]);
        assert_eq!(receipt["revision"], 2);
        assert_eq!(receipt["state_id"], 2);
        assert_ne!(receipt["state_id"], before["state_id"]);
        assert_eq!(receipt["history"]["undo_steps"], 2);
        assert_eq!(receipt["history"]["redo_steps"], 0);
        assert_eq!(
            receipt
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["patch", "revision", "state_id", "history"])
        );
        assert_eq!(
            *live.calls.lock().unwrap(),
            vec![json!({"sync":2,"patch":before["patch"]})]
        );
        assert_eq!(live.reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn stale_closed_and_exhausted_checkpoints_leave_all_history_and_backend_state_unchanged() {
        for failure in ["stale", "closed", "exhausted"] {
            let (mut session, live) = session();
            session
                .execute(
                    "set_parameter",
                    json!({"id":"source","parameter":"value","value":1,"history_group":"gesture"}),
                )
                .unwrap();
            session.execute("undo", json!({})).unwrap();
            if failure == "closed" {
                session.seal_for_close(session.revision()).unwrap();
            }
            if failure == "exhausted" {
                session.revision = u64::MAX;
            }
            let expected = if failure == "stale" {
                session.revision() - 1
            } else {
                session.revision()
            };
            let before = session.snapshot();
            let history: Vec<_> = session.history_state_ids().collect();
            live.calls.lock().unwrap().clear();
            assert!(
                session.record_layout_checkpoint(expected).is_err(),
                "{failure}"
            );
            assert_eq!(session.snapshot(), before);
            assert_eq!(session.history_state_ids().collect::<Vec<_>>(), history);
            assert!(live.calls.lock().unwrap().is_empty());
            assert_eq!(live.reads.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn checkpoint_undo_redo_and_branch_keep_graph_and_clear_redo_without_state_hooks() {
        let (mut session, live) = session();
        let before = session.snapshot();
        let first = session.record_layout_checkpoint(1).unwrap();
        let undone = session
            .execute("undo", json!({"expected_revision":2}))
            .unwrap();
        assert_eq!(undone["state_id"], before["state_id"]);
        assert_eq!(undone["patch"], before["patch"]);
        let redone = session
            .execute("redo", json!({"expected_revision":3}))
            .unwrap();
        assert_eq!(redone["state_id"], first["state_id"]);
        assert_eq!(redone["patch"], before["patch"]);
        session.execute("undo", json!({})).unwrap();
        assert!(session.can_redo());
        let branch = session.record_layout_checkpoint(5).unwrap();
        assert_eq!(branch["revision"], 6);
        assert_eq!(branch["state_id"], 6);
        assert!(!session.can_redo());
        assert!(session.execute("redo", json!({})).is_err());
        assert!(live
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.get("sync").is_some()));
        assert_eq!(live.reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn checkpoint_splits_parameter_history_group_even_when_gesture_name_repeats() {
        let (mut session, live) = session();
        for value in [1, 2] {
            session.execute("set_parameter",json!({"id":"source","parameter":"value","value":value,"history_group":"same-gesture"})).unwrap();
        }
        assert_eq!(session.undo.len(), 2);
        assert!(session.history_group.is_some());
        let checkpoint = session.record_layout_checkpoint(3).unwrap();
        assert!(session.history_group.is_none());
        session
            .execute(
                "set_parameter",
                json!({"id":"source","parameter":"value","value":3,"history_group":"same-gesture"}),
            )
            .unwrap();
        assert_eq!(session.undo.len(), 4);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.state_id(), checkpoint["state_id"].as_u64().unwrap());
        assert_eq!(session.patch.nodes[0].params["value"], 2.0);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.patch.nodes[0].params["value"], 2.0);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(session.patch.nodes[0].params["value"], 0.0);
        assert!(live
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.get("sync").is_some()));
    }

    #[test]
    fn layout_checkpoints_obey_shared_history_limit() {
        let (mut session, live) = session();
        let patch = session.snapshot()["patch"].clone();
        for _ in 0..HISTORY_LIMIT + 5 {
            session
                .record_layout_checkpoint(session.revision())
                .unwrap();
        }
        assert_eq!(session.undo.len(), HISTORY_LIMIT);
        let newest = session.state_id();
        for _ in 0..HISTORY_LIMIT {
            session.execute("undo", json!({})).unwrap();
        }
        assert!(!session.can_undo());
        assert_eq!(session.snapshot()["patch"], patch);
        for _ in 0..HISTORY_LIMIT {
            session.execute("redo", json!({})).unwrap();
        }
        assert_eq!(session.state_id(), newest);
        assert_eq!(session.snapshot()["patch"], patch);
        assert!(live
            .calls
            .lock()
            .unwrap()
            .iter()
            .all(|call| call.get("sync").is_some()));
        assert_eq!(live.reads.load(Ordering::Relaxed), 0);
    }
}
