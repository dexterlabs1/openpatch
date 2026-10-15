//! A coherent project save snapshot, independent of the live patch/history.
use super::*;

#[derive(Clone, Debug)]
pub struct ProjectPatchSnapshot {
    pub patch: Patch,
    pub revision: u64,
    pub state_id: u64,
    pub running: BTreeMap<String, bool>,
    pub holding: BTreeMap<String, bool>,
    pub saved_states: BTreeMap<String, SavedModuleState>,
    pub state_source: &'static str,
    pub frame: Option<u64>,
}

/// Lightweight Run and HOLD metadata for dirty observation; does not clone the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRunningSnapshot {
    pub revision: u64,
    pub state_id: u64,
    pub running: BTreeMap<String, bool>,
    pub holding: BTreeMap<String, bool>,
    pub saved_states: BTreeMap<String, SavedModuleState>,
    pub state_source: &'static str,
    pub frame: Option<u64>,
}

impl Session {
    /// Read current SEQ3 Run and PUSH HOLD from one coherent callback snapshot.
    /// Kept under the original method name for existing native callers.
    pub fn capture_project_running(
        &self,
        expected_revision: Option<u64>,
    ) -> Result<ProjectRunningSnapshot, String> {
        self.check_revision(expected_revision)?;
        let mut running = BTreeMap::new();
        let mut holding = BTreeMap::new();
        let mut saved_states = BTreeMap::new();
        let mut frame = None;
        let has_state = self
            .patch
            .nodes
            .iter()
            .any(|node| SavedModuleState::from_node(node).is_some());
        let snapshot = if has_state {
            self.live_module_events
                .as_ref()
                .map(|backend| backend.snapshot())
        } else {
            None
        };
        let state_source = if !has_state {
            "parameters"
        } else if snapshot.is_some() {
            "live"
        } else {
            "offline_reset"
        };
        for node in &self.patch.nodes {
            let Some(initial) = SavedModuleState::from_node(node) else {
                continue;
            };
            let state = if let Some(snapshot) = &snapshot {
                let (state, captured_frame) =
                    running::live_saved(snapshot, &node.id, self.revision, initial)?;
                if frame.is_some_and(|frame| frame != captured_frame) {
                    return Err(
                        "live project state has mixed callback frames; retry capture".into(),
                    );
                }
                frame = Some(captured_frame);
                state
            } else {
                initial
            };
            saved_states.insert(node.id.clone(), state);
            match state {
                SavedModuleState::Seq3Run(value) => {
                    running.insert(node.id.clone(), value);
                }
                SavedModuleState::PushHold(value) => {
                    holding.insert(node.id.clone(), value);
                }
                SavedModuleState::RandomValues { .. } | SavedModuleState::ShiftRegister { .. } => {}
            }
        }
        Ok(ProjectRunningSnapshot {
            revision: self.revision,
            state_id: self.state_id,
            running,
            holding,
            saved_states,
            state_source,
            frame,
        })
    }

    /// Project live persistent flags into a cloned patch without modifying history.
    pub fn capture_project_patch(
        &self,
        expected_revision: Option<u64>,
    ) -> Result<ProjectPatchSnapshot, String> {
        let captured = self.capture_project_running(expected_revision)?;
        let mut patch = self.patch.clone();
        for node in &mut patch.nodes {
            if let Some(state) = captured.saved_states.get(&node.id) {
                state.write_params(&mut node.params);
            }
        }
        Ok(ProjectPatchSnapshot {
            patch,
            revision: captured.revision,
            state_id: captured.state_id,
            running: captured.running,
            holding: captured.holding,
            saved_states: captured.saved_states,
            state_source: captured.state_source,
            frame: captured.frame,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Live {
        snapshot: Mutex<Value>,
        reads: AtomicUsize,
        writes: AtomicUsize,
    }
    impl LiveModuleEvents for Live {
        fn snapshot(&self) -> Value {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.snapshot.lock().unwrap().clone()
        }
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            unreachable!()
        }
        fn initialize_modules(&self, _: &[String]) {
            self.writes.fetch_add(1, Ordering::Relaxed);
        }
        fn load_module_running(&self, _: &BTreeMap<String, bool>) {
            self.writes.fetch_add(1, Ordering::Relaxed);
        }
        fn release_module_controls(&self, _: &[String]) {
            self.writes.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn session() -> Session {
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        session.execute("replace_patch",json!({"patch":{"version":2,"sample_rate":48000,"nodes":[
            {"id":"a","kind":"seq3","params":{"running":1,"tempo":60,"row1_1":4}},
            {"id":"b","kind":"seq3","bypassed":true,"params":{"running":0,"gate8":0}},
            {"id":"vco","kind":"multi_vco","params":{"frequency":440}}
        ],"cables":[{"id":"route","from":"a","output":"cv1","to":"vco","input":"pitch"}],"output":"vco","output_port":"sine"}})).unwrap();
        session
    }

    fn snapshot(revision: u64) -> Value {
        json!({"modules":{
            "a":{"kind":"seq3","available":true,"revision":revision,"frame":128,"state":{"running":false}},
            "b":{"kind":"seq3","available":true,"revision":revision,"frame":128,"state":{"running":true}}
        }})
    }

    fn attach(session: &mut Session) -> Arc<Live> {
        let live = Arc::new(Live::default());
        *live.snapshot.lock().unwrap() = snapshot(session.revision());
        session.set_live_module_events(Some(live.clone()));
        live
    }

    #[test]
    fn captures_opposite_live_run_in_one_snapshot_without_any_session_or_runtime_mutation() {
        let mut session = session();
        let live = attach(&mut session);
        let before = session.snapshot();
        let history: Vec<_> = session.history_state_ids().collect();
        let backend = live.snapshot.lock().unwrap().clone();
        let captured = session.capture_project_patch(Some(1)).unwrap();
        assert_eq!(live.reads.load(Ordering::Relaxed), 1);
        assert_eq!(captured.revision, 1);
        assert_eq!(captured.state_id, session.state_id());
        assert_eq!(captured.frame, Some(128));
        assert_eq!(captured.state_source, "live");
        assert_eq!(
            captured.running,
            BTreeMap::from([("a".into(), false), ("b".into(), true)])
        );
        let mut expected = session.patch().clone();
        expected.nodes[0].params.insert("running".into(), 0.0);
        expected.nodes[1].params.insert("running".into(), 1.0);
        assert_eq!(
            serde_json::to_value(&captured.patch).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert!(captured.patch.nodes[1].bypassed);
        assert_eq!(session.snapshot(), before);
        assert_eq!(session.history_state_ids().collect::<Vec<_>>(), history);
        assert_eq!(*live.snapshot.lock().unwrap(), backend);
        assert_eq!(live.writes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn stale_missing_unavailable_nonboolean_and_mixed_live_frames_are_rejected_atomically() {
        let mut session = session();
        let live = attach(&mut session);
        let before = session.snapshot();
        assert!(session.capture_project_patch(Some(0)).is_err());
        assert_eq!(live.reads.load(Ordering::Relaxed), 0);
        for field in [
            "unavailable",
            "missing",
            "stale",
            "running",
            "frame",
            "mixed",
        ] {
            let mut value = snapshot(1);
            match field {
                "unavailable" => value["modules"]["b"]["available"] = json!(false),
                "missing" => {
                    value["modules"].as_object_mut().unwrap().remove("b");
                }
                "stale" => value["modules"]["b"]["revision"] = json!(0),
                "running" => value["modules"]["b"]["state"]["running"] = json!(1),
                "frame" => {
                    value["modules"]["b"]
                        .as_object_mut()
                        .unwrap()
                        .remove("frame");
                }
                "mixed" => value["modules"]["b"]["frame"] = json!(256),
                _ => unreachable!(),
            }
            *live.snapshot.lock().unwrap() = value;
            assert!(session.capture_project_patch(None).is_err(), "{field}");
            assert_eq!(session.snapshot(), before);
        }
        assert_eq!(live.reads.load(Ordering::Relaxed), 6);
        assert_eq!(live.writes.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn reused_id_cannot_capture_previous_incarnations_revision() {
        let mut session = session();
        let live = attach(&mut session);
        session.execute("remove_module", json!({"id":"a"})).unwrap();
        session
            .execute(
                "add_module",
                json!({"id":"a","kind":"seq3","params":{"running":1}}),
            )
            .unwrap();
        // The registry still contains a plausible old ID/value, but its revision
        // belongs to the removed instance and must never be captured as current.
        live.snapshot.lock().unwrap()["modules"]["b"]["revision"] = json!(3);
        let reads = live.reads.load(Ordering::Relaxed);
        assert!(session
            .capture_project_patch(Some(3))
            .unwrap_err()
            .contains("module 'a'"));
        assert_eq!(live.reads.load(Ordering::Relaxed), reads + 1);
        *live.snapshot.lock().unwrap() = snapshot(3);
        let captured = session.capture_project_patch(Some(3)).unwrap();
        assert!(!captured.running["a"]);
    }

    #[test]
    fn offline_capture_uses_initial_defaults_and_no_sequence_skips_backend_entirely() {
        let mut session = session();
        session.patch.nodes[0].params.remove("running");
        let before = session.snapshot();
        let captured = session.capture_project_patch(Some(1)).unwrap();
        assert_eq!(captured.state_source, "offline_reset");
        assert_eq!(captured.frame, None);
        assert_eq!(
            captured.running,
            BTreeMap::from([("a".into(), true), ("b".into(), false)])
        );
        assert_eq!(captured.patch.nodes[0].params["running"], 1.0);
        assert_eq!(session.snapshot(), before);
        let mut parameters_only = session.patch().clone();
        parameters_only.nodes.retain(|node| node.kind != "seq3");
        parameters_only.cables.clear();
        session.replace_patch(parameters_only, Some(1)).unwrap();
        let live = attach(&mut session);
        *live.snapshot.lock().unwrap() = json!({"coherent":false});
        let lightweight = session.capture_project_running(Some(2)).unwrap();
        assert_eq!(lightweight.state_source, "parameters");
        assert!(lightweight.running.is_empty());
        let captured = session.capture_project_patch(Some(2)).unwrap();
        assert_eq!(captured.state_source, "parameters");
        assert!(captured.running.is_empty());
        assert_eq!(captured.frame, None);
        assert_eq!(live.reads.load(Ordering::Relaxed), 0);
        assert_eq!(
            serde_json::to_value(captured.patch).unwrap(),
            session.snapshot()["patch"]
        );
    }

    #[test]
    fn lightweight_and_full_capture_share_validation_and_metadata() {
        let mut session = session();
        let live = attach(&mut session);
        let before = session.snapshot();
        let observed = session.capture_project_running(Some(1)).unwrap();
        let captured = session.capture_project_patch(Some(1)).unwrap();
        assert_eq!(live.reads.load(Ordering::Relaxed), 2);
        assert_eq!(observed.running, captured.running);
        assert_eq!(observed.revision, captured.revision);
        assert_eq!(observed.state_id, captured.state_id);
        assert_eq!(observed.state_source, captured.state_source);
        assert_eq!(observed.frame, captured.frame);
        // Plausible, current, same-frame meters cannot override an explicit
        // incoherence marker from a contended callback publication.
        live.snapshot.lock().unwrap()["coherent"] = json!(false);
        assert!(session
            .capture_project_running(Some(1))
            .unwrap_err()
            .contains("not coherent"));
        assert!(session
            .capture_project_patch(Some(1))
            .unwrap_err()
            .contains("not coherent"));
        live.snapshot.lock().unwrap()["coherent"] = json!(true);
        assert_eq!(session.capture_project_running(Some(1)).unwrap(), observed);
        assert_eq!(session.snapshot(), before);
        assert_eq!(live.writes.load(Ordering::Relaxed), 0);
        let reads = live.reads.load(Ordering::Relaxed);
        assert!(session.capture_project_running(Some(0)).is_err());
        assert_eq!(live.reads.load(Ordering::Relaxed), reads);
        session.set_live_module_events(None);
        let observed = session.capture_project_running(None).unwrap();
        assert_eq!(observed.state_source, "offline_reset");
        assert_eq!(
            observed.running,
            BTreeMap::from([("a".into(), true), ("b".into(), false)])
        );
    }

    #[test]
    fn mcp_capture_keeps_live_backend_and_exposes_strict_read_only_schema() {
        let mut session = session();
        let live = attach(&mut session);
        let before = session.snapshot();
        let shared = session.into_shared();
        let captured = execute_shared(
            &shared,
            "capture_project_patch",
            json!({"expected_revision":1}),
        )
        .unwrap();
        assert_eq!(captured["state_source"], "live");
        assert_eq!(captured["running"]["a"], false);
        assert_eq!(captured["patch"]["nodes"][0]["params"]["running"], 0.0);
        assert_eq!(live.reads.load(Ordering::Relaxed), 1);
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        for args in [
            json!({"expected_revision":0}),
            json!({"expected_revision":true}),
            json!({"unknown":1}),
        ] {
            assert!(execute_shared(&shared, "capture_project_patch", args).is_err());
        }
        let tools = tools();
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "capture_project_patch")
            .unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
        assert_eq!(shared.lock().unwrap().snapshot(), before);
    }
}
