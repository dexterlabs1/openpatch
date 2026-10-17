//! A coherent project save snapshot, independent of the live patch/history.
use super::*;
type EffectiveParameters = BTreeMap<String, BTreeMap<String, f32>>;

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
        self.capture_project_state(expected_revision, false)
            .map(|(state, _)| state)
    }

    fn capture_project_state(
        &self,
        expected_revision: Option<u64>,
        with_parameters: bool,
    ) -> Result<(ProjectRunningSnapshot, EffectiveParameters), String> {
        self.check_midi_cc_capture()?;
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
        let has_live_parameters = with_parameters
            && self
                .live_module_events
                .as_ref()
                .is_some_and(|backend| backend.supports_live_parameters())
            && self
                .patch
                .nodes
                .iter()
                .any(|node| !openpatch_core::runtime_parameter_names(&node.kind).is_empty());
        let snapshot = if has_state || has_live_parameters {
            self.live_module_events
                .as_ref()
                .map(|backend| backend.snapshot())
        } else {
            None
        };
        let state_source = if snapshot.is_some() {
            "live"
        } else if has_state {
            "offline_reset"
        } else {
            "parameters"
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
            super::rackmcp_transport::validate_selection(node, state)?;
            saved_states.insert(node.id.clone(), state);
            match state {
                SavedModuleState::Seq3Run(value)
                | SavedModuleState::StockSeq3 { running: value, .. } => {
                    running.insert(node.id.clone(), value);
                }
                SavedModuleState::PushHold(value) => {
                    holding.insert(node.id.clone(), value);
                }
                SavedModuleState::RackMcpSelection(_)
                | SavedModuleState::StockSeq3Fields { .. }
                | SavedModuleState::StockQuantizerNotes(_)
                | SavedModuleState::RandomValues { .. }
                | SavedModuleState::ShiftRegister { .. }
                | SavedModuleState::MidiCc { .. }
                | SavedModuleState::MidiCcMappings(_)
                | SavedModuleState::MidiGateMappings(_) => {}
            }
        }
        let mut parameters = EffectiveParameters::new();
        if has_live_parameters {
            let snapshot = snapshot
                .as_ref()
                .expect("live parameter backend was captured");
            if snapshot["coherent"].as_bool() == Some(false) {
                return Err("live project parameters are not coherent; retry capture".into());
            }
            for node in &self.patch.nodes {
                let names = openpatch_core::runtime_parameter_names(&node.kind);
                if names.is_empty() {
                    continue;
                }
                let module = &snapshot["modules"][&node.id];
                if module["available"] != true
                    || module["revision"].as_u64() != Some(self.revision)
                    || module["kind"].as_str() != Some(node.kind.as_str())
                {
                    return Err(format!(
                        "live parameters unavailable for module '{}'; wait for revision {}",
                        node.id, self.revision
                    ));
                }
                let captured_frame = module["frame"]
                    .as_u64()
                    .ok_or_else(|| format!("live parameter frame unavailable for '{}'", node.id))?;
                if frame.is_some_and(|seen| seen != captured_frame) {
                    return Err(
                        "live project state has mixed callback frames; retry capture".into(),
                    );
                }
                frame = Some(captured_frame);
                let mut values = BTreeMap::new();
                for &name in names {
                    let value = module["parameters"][name].as_f64().ok_or_else(|| {
                        format!("live parameter '{name}' unavailable for '{}'", node.id)
                    })?;
                    let descriptor = openpatch_core::runtime_parameter_descriptor(&node.kind, name)
                        .expect("declared native capability");
                    let raw = value as f32;
                    if !raw.is_finite()
                        || raw < descriptor.min
                        || raw > descriptor.max
                        || (descriptor.snap && raw.fract() != 0.0)
                    {
                        return Err(format!("invalid live parameter '{name}' for '{}'", node.id));
                    }
                    values.insert(name.to_owned(), raw);
                }
                parameters.insert(node.id.clone(), values);
            }
        }
        self.check_midi_cc_capture()?;
        Ok((
            ProjectRunningSnapshot {
                revision: self.revision,
                state_id: self.state_id,
                running,
                holding,
                saved_states,
                state_source,
                frame,
            },
            parameters,
        ))
    }

    /// Project live persistent flags into a cloned patch without modifying history.
    pub fn capture_project_patch(
        &self,
        expected_revision: Option<u64>,
    ) -> Result<ProjectPatchSnapshot, String> {
        self.require_native_document()?;
        self.capture_native_project_patch(expected_revision)
    }

    pub(super) fn capture_native_project_patch(
        &self,
        expected_revision: Option<u64>,
    ) -> Result<ProjectPatchSnapshot, String> {
        let (captured, parameters) = self.capture_project_state(expected_revision, true)?;
        let mut patch = self.patch.clone();
        for node in &mut patch.nodes {
            if let Some(state) = captured.saved_states.get(&node.id) {
                state.write_node(node);
            }
            if let Some(values) = parameters.get(&node.id) {
                node.params
                    .extend(values.iter().map(|(name, value)| (name.clone(), *value)));
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Live {
        snapshot: Mutex<Value>,
        reads: AtomicUsize,
        writes: AtomicUsize,
        parameters: AtomicBool,
    }
    impl LiveModuleEvents for Live {
        fn supports_live_parameters(&self) -> bool {
            self.parameters.load(Ordering::Relaxed)
        }
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
        session.set_live_module_events(Some(live.clone())).unwrap();
        live
    }

    fn parameter_session() -> (Session, Arc<Live>) {
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        session
            .execute(
                "replace_patch",
                json!({"patch":{"version":2,"sample_rate":48000,"nodes":[
            {"id":"seq","kind":"seq3","params":{"running":1}},
            {"id":"vca","kind":"stock_vca","params":{"level":1}},
            {"id":"filter","kind":"rack264_vcf","params":{"frequency":0.5}}
        ],"cables":[],"output":"vca","output_port":"out"}}),
            )
            .unwrap();
        let live = Arc::new(Live::default());
        live.parameters.store(true, Ordering::Relaxed);
        *live.snapshot.lock().unwrap() = json!({"coherent":true,"modules":{
            "seq":{"kind":"seq3","available":true,"revision":1,"frame":128,"state":{"running":false}},
            "vca":{"kind":"stock_vca","available":true,"revision":1,"frame":128,"parameters":{"level":0.25}},
            "filter":{"kind":"rack264_vcf","available":true,"revision":1,"frame":128,"parameters":{"frequency":0.3}}
        }});
        session.set_live_module_events(Some(live.clone())).unwrap();
        (session, live)
    }

    #[test]
    fn coherent_effective_parameters_overlay_saved_clone_without_changing_history() {
        let (session, live) = parameter_session();
        let before = session.snapshot();
        let captured = session.capture_project_patch(Some(1)).unwrap();
        assert_eq!(captured.patch.nodes[1].params["level"], 0.25);
        assert_eq!(captured.patch.nodes[2].params["frequency"], 0.3);
        assert!(!captured.running["seq"]);
        assert_eq!(captured.frame, Some(128));
        assert_eq!(captured.state_source, "live");
        assert_eq!(
            live.reads.load(Ordering::Relaxed),
            1,
            "state/parameter capture split into separate observations"
        );
        assert_eq!(live.writes.load(Ordering::Relaxed), 0);
        assert_eq!(session.snapshot(), before);
        let saved = serde_json::to_value(&captured.patch).unwrap();
        let reopened: Patch = serde_json::from_value(saved).unwrap();
        let engine = reopened.compile().unwrap();
        let handle = engine.resolve_parameter("vca", "level").unwrap();
        assert_eq!(engine.parameter_value(handle), Some(0.25));
        let handle = engine.resolve_parameter("filter", "frequency").unwrap();
        assert_eq!(engine.parameter_value(handle), Some(0.3));
    }

    #[test]
    fn effective_parameter_capture_rejects_stale_mixed_missing_or_invalid_publications() {
        let (session, live) = parameter_session();
        let valid = live.snapshot.lock().unwrap().clone();
        let before = session.snapshot();
        for fault in [
            "coherent",
            "available",
            "revision",
            "kind",
            "frame",
            "mixed",
            "missing",
            "bounds",
            "null",
        ] {
            let mut value = valid.clone();
            match fault {
                "coherent" => value["coherent"] = json!(false),
                "available" => value["modules"]["vca"]["available"] = json!(false),
                "revision" => value["modules"]["vca"]["revision"] = json!(0),
                "kind" => value["modules"]["vca"]["kind"] = json!("rack264_vcf"),
                "frame" => {
                    value["modules"]["vca"]
                        .as_object_mut()
                        .unwrap()
                        .remove("frame");
                }
                "mixed" => value["modules"]["filter"]["frame"] = json!(256),
                "missing" => {
                    value["modules"]["vca"]["parameters"]
                        .as_object_mut()
                        .unwrap()
                        .remove("level");
                }
                "bounds" => value["modules"]["vca"]["parameters"]["level"] = json!(1.1),
                "null" => value["modules"]["vca"]["parameters"]["level"] = Value::Null,
                _ => unreachable!(),
            }
            *live.snapshot.lock().unwrap() = value;
            assert!(
                session.capture_project_patch(None).is_err(),
                "accepted {fault}"
            );
            assert_eq!(session.snapshot(), before);
        }
    }

    #[test]
    fn lightweight_dirty_capture_ignores_effective_parameter_changes_and_legacy_backends_keep_authored_values(
    ) {
        let (session, live) = parameter_session();
        let first = session.capture_project_running(None).unwrap();
        live.snapshot.lock().unwrap()["modules"]["vca"]["parameters"]["level"] = json!(0.7);
        assert_eq!(session.capture_project_running(None).unwrap(), first);
        live.parameters.store(false, Ordering::Relaxed);
        let capture = session.capture_project_patch(None).unwrap();
        assert_eq!(capture.patch.nodes[1].params["level"], 1.0);
        assert_eq!(capture.patch.nodes[2].params["frequency"], 0.5);
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
        session.set_live_module_events(None).unwrap();
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
