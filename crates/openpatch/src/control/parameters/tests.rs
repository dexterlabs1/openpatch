use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Live {
    state: Mutex<Value>,
    assigned: Mutex<Vec<ParameterValues>>,
    reads: AtomicUsize,
}
impl LiveModuleEvents for Live {
    fn supports_live_parameters(&self) -> bool {
        true
    }
    fn snapshot(&self) -> Value {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.state.lock().unwrap().clone()
    }
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        unreachable!()
    }
    fn sync_patch(&self, patch: &Patch, revision: u64) {
        self.sync_patch_with_parameters(patch, revision, &ParameterValues::new());
    }
    fn sync_patch_with_parameters(
        &self,
        patch: &Patch,
        revision: u64,
        assignments: &ParameterValues,
    ) {
        self.assigned.lock().unwrap().push(assignments.clone());
        let mut state = self.state.lock().unwrap();
        let mut modules = serde_json::Map::new();
        for node in &patch.nodes {
            let mut values = serde_json::Map::new();
            for &key in runtime_parameter_names(&node.kind) {
                let value = assignments
                    .get(&node.id)
                    .and_then(|v| v.get(key))
                    .copied()
                    .or_else(|| {
                        state["modules"][&node.id]["parameters"][key]
                            .as_f64()
                            .map(|v| v as f32)
                    })
                    .unwrap_or_else(|| authored(node, key));
                values.insert(key.into(), json!(value));
            }
            modules.insert(node.id.clone(),json!({"available":true,"kind":node.kind,"revision":revision,"frame":128,"parameters":values}));
        }
        *state = json!({"coherent":true,"modules":modules});
    }
}
fn fixture() -> (Session, Arc<Live>) {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"sample_rate":48000,"nodes":[
        {"id":"signal","kind":"poly_constant","params":{"v1":5}},
        {"id":"vca","kind":"stock_vca","params":{"level":1}},
        {"id":"filter","kind":"rack264_vcf","params":{"frequency":0.5}}
    ],"cables":[],"output":"vca","output_port":"out"}}),
        )
        .unwrap();
    let live = Arc::new(Live::default());
    *live.state.lock().unwrap() = json!({"modules":{
        "vca":{"parameters":{"level":0.25}},"filter":{"parameters":{"frequency":0.3}}
    }});
    session.set_live_module_events(Some(live.clone())).unwrap();
    (session, live)
}
fn value(live: &Live, id: &str, key: &str) -> f32 {
    live.state.lock().unwrap()["modules"][id]["parameters"][key]
        .as_f64()
        .unwrap() as f32
}

#[test]
fn explicit_equal_assignment_undo_redo_restore_effective_before_value_only() {
    let (mut session, live) = fixture();
    session
        .execute(
            "set_parameter",
            json!({"id":"vca","parameter":"level","value":1}),
        )
        .unwrap();
    assert_eq!(value(&live, "vca", "level"), 1.0);
    assert_eq!(
        live.assigned.lock().unwrap().last().unwrap()["vca"]["level"],
        1.0
    );
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.patch.nodes[1].params["level"], 0.25);
    assert_eq!(value(&live, "vca", "level"), 0.25);
    assert_eq!(value(&live, "filter", "frequency"), 0.3);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 1.0);
    assert_eq!(value(&live, "filter", "frequency"), 0.3);
}

#[test]
fn ordinary_edits_and_undo_send_no_unrelated_parameter_assignments() {
    let (mut session, live) = fixture();
    let reads = live.reads.load(Ordering::Relaxed);
    session
        .execute(
            "set_parameter",
            json!({"id":"signal","parameter":"v1","value":3}),
        )
        .unwrap();
    assert!(live.assigned.lock().unwrap().last().unwrap().is_empty());
    session.execute("undo", json!({})).unwrap();
    assert!(live.assigned.lock().unwrap().last().unwrap().is_empty());
    assert_eq!(value(&live, "vca", "level"), 0.25);
    assert_eq!(live.reads.load(Ordering::Relaxed), reads);
}

#[test]
fn batch_and_grouped_gestures_keep_original_effective_undo_and_final_redo() {
    let (mut session, live) = fixture();
    for value in [0.5, 0.75] {
        session
            .execute(
                "set_parameter",
                json!({"id":"vca","parameter":"level","value":value,"history_group":"gain"}),
            )
            .unwrap();
    }
    session.execute("undo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 0.25);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 0.75);
    let (mut session, live) = fixture();
    session
        .execute(
            "apply_edits",
            json!({"edits":[
                {"tool":"set_parameter","arguments":{"id":"vca","parameter":"level","value":0.5}},
                {"tool":"set_parameter","arguments":{"id":"vca","parameter":"level","value":0.75}}
            ]}),
        )
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 0.25);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 0.75);
}

#[test]
fn initialize_and_preset_capture_effective_values_even_when_authored_values_equal() {
    for tool in ["initialize_modules", "apply_module_preset"] {
        let (mut session, live) = fixture();
        let args = if tool == "initialize_modules" {
            json!({"ids":["vca"]})
        } else {
            json!({"id":"vca","preset":{"format":"openpatch-module-preset","version":1,"kind":"stock_vca","params":{"level":1}}})
        };
        session.execute(tool, args).unwrap();
        assert_eq!(value(&live, "vca", "level"), 1.0);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(value(&live, "vca", "level"), 0.25);
        session.execute("redo", json!({})).unwrap();
        assert_eq!(value(&live, "vca", "level"), 1.0);
    }
}

#[test]
fn deletion_and_full_replacement_undo_restore_what_was_heard() {
    let (mut session, live) = fixture();
    session
        .execute("remove_module", json!({"id":"vca"}))
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 0.25);
    let (mut session, live) = fixture();
    let replacement = session.patch.clone();
    session.replace_patch(replacement, Some(1)).unwrap();
    assert_eq!(value(&live, "vca", "level"), 1.0);
    assert_eq!(value(&live, "filter", "frequency"), 0.5);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(value(&live, "vca", "level"), 0.25);
    assert_eq!(value(&live, "filter", "frequency"), 0.3);
}

#[test]
fn preview_failures_and_stale_observations_do_not_mutate_assignment_authority() {
    let (mut session, live) = fixture();
    let before = session.snapshot();
    let assignments = live.assigned.lock().unwrap().clone();
    session.execute("preview_edits",json!({"edits":[{"tool":"set_parameter","arguments":{"id":"vca","parameter":"level","value":0.5}}]})).unwrap();
    assert_eq!(session.snapshot(), before);
    assert_eq!(*live.assigned.lock().unwrap(), assignments);
    assert!(session
        .execute(
            "set_parameter",
            json!({"id":"vca","parameter":"level","value":2})
        )
        .is_err());
    live.state.lock().unwrap()["modules"]["vca"]["revision"] = json!(0);
    assert!(session
        .execute(
            "set_parameter",
            json!({"id":"vca","parameter":"level","value":0.5})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
    assert_eq!(*live.assigned.lock().unwrap(), assignments);
}
