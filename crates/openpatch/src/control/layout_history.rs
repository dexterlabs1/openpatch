//! Shared history boundaries for native rack layout gestures.
use super::*;

/// Highest rack row an agent may request for a new module.
pub(super) const MAX_LAYOUT_ROW: u32 = 63;

/// Parse the optional `row` placement of an `add_module` edit.
pub(super) fn requested_row(args: &Value) -> Result<Option<u32>, String> {
    let Some(row) = args.get("row") else {
        return Ok(None);
    };
    row.as_u64()
        .filter(|row| *row <= u64::from(MAX_LAYOUT_ROW))
        .map(|row| Some(row as u32))
        .ok_or_else(|| format!("row must be an integer from 0 to {MAX_LAYOUT_ROW}"))
}

impl Session {
    /// Session owns the graph and the desktop owns geometry, so a requested
    /// row is presentation metadata, retained with graph history so an unseen
    /// add can still be placed after Undo/Redo. Retired incarnations never lend
    /// their row to a later module that happens to reuse the same string ID.
    pub(super) fn record_layout_placements(
        &mut self,
        placements: &BTreeMap<String, u32>,
        retired: &[String],
    ) {
        let patch = &self.patch;
        self.layout_rows
            .retain(|id, _| !retired.contains(id) && patch.nodes.iter().any(|node| &node.id == id));
        self.layout_placements
            .retain(|id, _| !retired.contains(id) && patch.nodes.iter().any(|node| &node.id == id));
        for (id, row) in placements {
            if patch.nodes.iter().any(|node| &node.id == id) {
                self.layout_rows.insert(id.clone(), *row);
                self.layout_placements.insert(id.clone(), *row);
            }
        }
    }

    pub(super) fn restore_layout_placements(
        &mut self,
        previous: &Patch,
        previous_rows: &BTreeMap<String, u32>,
        retired: &[String],
    ) {
        self.layout_placements.retain(|id, _| {
            !retired.contains(id) && self.patch.nodes.iter().any(|node| &node.id == id)
        });
        for (id, row) in &self.layout_rows {
            if retired.contains(id)
                || !previous.nodes.iter().any(|node| &node.id == id)
                || previous_rows.get(id) != Some(row)
            {
                self.layout_placements.insert(id.clone(), *row);
            }
        }
    }

    /// Drain the rows requested by committed `add_module` edits. The host
    /// appends each module to the end of its row the first time it observes it.
    /// Undo/Redo may redeliver a restored incarnation's hint. A host's stored
    /// layout for that history state takes precedence over these fallback rows.
    pub fn take_layout_placements(&mut self) -> BTreeMap<String, u32> {
        std::mem::take(&mut self.layout_placements)
    }

    /// Record one rack-layout history state without changing the graph or
    /// retiring DSP identities. The host stores geometry under the returned
    /// state_id; ordinary shared undo/redo traverses the same boundary.
    pub fn record_layout_checkpoint(&mut self, expected_revision: u64) -> Result<Value, String> {
        // Legacy host move receipts carry their final positions separately.
        // Keep capture fenced until that exact placement reaches the host,
        // including when another MCP graph edit commits before it is drawn.
        let generation = if self.layout_host_attached {
            Some(
                self.layout_generation
                    .checked_add(1)
                    .ok_or("layout generation exhausted")?,
            )
        } else {
            None
        };
        self.commit(
            self.patch.clone(),
            Some(expected_revision),
            None,
            &EditEffects::default(),
        )?;
        if let Some(generation) = generation {
            self.layout_generation = generation;
            self.layout_pending = true;
        }
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

    #[test]
    fn add_module_rows_queue_for_the_host_once_and_follow_the_final_graph() {
        let (mut session, _) = session();
        session
            .execute("add_module", json!({"id":"a","kind":"constant","row":2}))
            .unwrap();
        session
            .execute(
                "apply_edits",
                json!({"edits":[
                    {"tool":"add_module","arguments":{"id":"b","kind":"constant","row":1}},
                    {"tool":"add_module","arguments":{"id":"gone","kind":"constant","row":3}},
                    {"tool":"remove_module","arguments":{"id":"gone"}},
                    {"tool":"add_module","arguments":{"id":"plain","kind":"constant"}}]}),
            )
            .unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2), ("b".into(), 1), ("plain".into(), 0)])
        );
        assert!(session.take_layout_placements().is_empty());

        // A later removal drops a request the host has not observed yet.
        session
            .execute("add_module", json!({"id":"late","kind":"constant","row":4}))
            .unwrap();
        session
            .execute("remove_module", json!({"id":"late"}))
            .unwrap();
        assert!(session.take_layout_placements().is_empty());
    }

    #[test]
    fn unobserved_rows_survive_history_but_do_not_reposition_unchanged_modules() {
        let (mut session, _) = session();
        session
            .execute("add_module", json!({"id":"a","kind":"constant","row":2}))
            .unwrap();
        session.execute("undo", json!({})).unwrap();
        assert!(session.take_layout_placements().is_empty());
        session.execute("redo", json!({})).unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2)])
        );
        session
            .execute(
                "set_parameter",
                json!({"id":"a","parameter":"value","value":2.0}),
            )
            .unwrap();
        session.execute("undo", json!({})).unwrap();
        assert!(
            session.take_layout_placements().is_empty(),
            "parameter Undo must preserve the host's manually moved geometry"
        );
        session.execute("redo", json!({})).unwrap();
        assert!(session.take_layout_placements().is_empty());
        session.execute("remove_module", json!({"id":"a"})).unwrap();
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2)])
        );
        session.execute("redo", json!({})).unwrap();
        assert!(session.take_layout_placements().is_empty());
    }

    #[test]
    fn pending_rows_belong_to_incarnations_including_default_row_replacements() {
        let (mut session, _) = session();
        session
            .execute("add_module", json!({"id":"a","kind":"constant","row":2}))
            .unwrap();
        session
            .execute(
                "apply_edits",
                json!({"edits":[
                    {"tool":"remove_module","arguments":{"id":"a"}},
                    {"tool":"add_module","arguments":{"id":"a","kind":"constant"}}
                ]}),
            )
            .unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 0)])
        );
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2)])
        );
        session.execute("redo", json!({})).unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 0)])
        );

        session.execute("undo", json!({})).unwrap();
        let replacement = session.patch().clone();
        session.replace_patch(replacement, None).unwrap();
        assert!(
            session.take_layout_placements().is_empty(),
            "a replacement document cannot inherit the old incarnation's pending row"
        );
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2)])
        );
    }

    #[test]
    fn rejected_and_preview_batches_do_not_consume_or_replace_pending_rows() {
        let (mut session, _) = session();
        session
            .execute("add_module", json!({"id":"a","kind":"constant","row":2}))
            .unwrap();
        let replacement = json!({"edits":[
            {"tool":"remove_module","arguments":{"id":"a"}},
            {"tool":"add_module","arguments":{"id":"a","kind":"constant","row":3}}
        ]});
        session.execute("preview_edits", replacement).unwrap();
        assert!(session
            .execute(
                "apply_edits",
                json!({"edits":[
                    {"tool":"remove_module","arguments":{"id":"a"}},
                    {"tool":"add_module","arguments":{"id":"a","kind":"constant","row":3}},
                    {"tool":"add_module","arguments":{"id":"bad","kind":"constant","row":64}}
                ]})
            )
            .is_err());
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2)])
        );
        session.execute("undo", json!({})).unwrap();
        assert!(session.take_layout_placements().is_empty());
    }

    #[test]
    fn undelivered_row_survives_an_ordinary_parameter_history_transition() {
        let (mut session, _) = session();
        session
            .execute("add_module", json!({"id":"a","kind":"constant","row":2}))
            .unwrap();
        session
            .execute(
                "set_parameter",
                json!({"id":"a","parameter":"value","value":2.0}),
            )
            .unwrap();
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.take_layout_placements(),
            BTreeMap::from([("a".into(), 2)])
        );
    }

    #[test]
    fn invalid_rows_reject_the_whole_edit_and_preview_records_nothing() {
        let (mut session, _) = session();
        let revision = session.revision();
        for row in [json!(-1), json!(64), json!(1.5), json!("1")] {
            let error = session
                .execute("add_module", json!({"id":"a","kind":"constant","row":row}))
                .unwrap_err();
            assert_eq!(error, "row must be an integer from 0 to 63");
        }
        session
            .execute(
                "preview_edits",
                json!({"edits":[{"tool":"add_module","arguments":{"id":"a","kind":"constant","row":1}}]}),
            )
            .unwrap();
        assert_eq!(session.revision(), revision);
        assert!(session.take_layout_placements().is_empty());
    }
}
