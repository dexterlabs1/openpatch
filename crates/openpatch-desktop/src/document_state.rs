//! Content-based saved state, independent of revision/history identities.
//! Refreshed when graph, persisted rack layout, or effective persistent module state changes.
use crate::{project::Document, rack::Rack};
use openpatch_core::{Patch, SavedModuleState};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct DocumentState {
    current: Value,
    baseline: Option<Value>,
    observed: Option<(u64, u64)>,
    dirty: bool,
    saved_states: BTreeMap<String, SavedModuleState>,
    known: bool,
}
impl DocumentState {
    pub fn new(patch: &Patch, revision: u64, rack: &Rack) -> Self {
        let saved_states = configured_saved_states(patch);
        let current = saved_content(patch, rack.layout(), &saved_states);
        Self {
            baseline: Some(current.clone()),
            current,
            observed: Some((revision, rack.layout_revision())),
            dirty: false,
            saved_states,
            known: true,
        }
    }
    #[cfg(test)]
    pub fn observe(&mut self, patch: &Patch, revision: u64, rack: &Rack) -> bool {
        self.observe_runtime(
            patch,
            revision,
            rack,
            Some((configured_running(patch), configured_holding(patch))),
        )
    }
    #[cfg(test)]
    pub fn observe_running(
        &mut self,
        patch: &Patch,
        revision: u64,
        rack: &Rack,
        running: Option<BTreeMap<String, bool>>,
    ) -> bool {
        self.observe_runtime(
            patch,
            revision,
            rack,
            running.map(|running| (running, configured_holding(patch))),
        )
    }
    #[cfg(test)]
    pub fn observe_runtime(
        &mut self,
        patch: &Patch,
        revision: u64,
        rack: &Rack,
        states: Option<(BTreeMap<String, bool>, BTreeMap<String, bool>)>,
    ) -> bool {
        let states = states.map(|(running, holding)| {
            let mut states = configured_saved_states(patch);
            states.extend(
                running
                    .into_iter()
                    .map(|(id, value)| (id, SavedModuleState::Seq3Run(value))),
            );
            states.extend(
                holding
                    .into_iter()
                    .map(|(id, value)| (id, SavedModuleState::PushHold(value))),
            );
            states
        });
        self.observe_saved_runtime(patch, revision, rack, states)
    }
    pub fn observe_saved_runtime(
        &mut self,
        patch: &Patch,
        revision: u64,
        rack: &Rack,
        states: Option<BTreeMap<String, SavedModuleState>>,
    ) -> bool {
        let Some(saved_states) = states else {
            let changed = self.known;
            self.known = false;
            self.observed = None;
            return changed;
        };
        let key = (revision, rack.layout_revision());
        if self.known && self.observed == Some(key) && self.saved_states == saved_states {
            return false;
        }
        let was_unknown = !self.known;
        self.known = true;
        self.observed = Some(key);
        let next = saved_content(patch, rack.layout(), &saved_states);
        self.saved_states = saved_states;
        let changed = was_unknown || self.current != next;
        self.current = next;
        self.dirty = self.baseline.as_ref() != Some(&self.current);
        changed
    }
    pub fn dirty(&self) -> bool {
        !self.known || self.dirty
    }
    pub fn matches_saved(&self, document: &Document) -> bool {
        self.baseline.as_ref() == Some(&document_content(document))
    }
    pub fn mark_saved(&mut self, document: &Document) {
        self.baseline = Some(document_content(document));
        self.dirty = self.baseline.as_ref() != Some(&self.current);
    }
    pub fn mark_unsaved(&mut self) {
        self.baseline = None;
        self.observed = None;
        self.dirty = true;
    }
    pub fn invalidate(&mut self) {
        self.observed = None;
    }
}

#[cfg(test)]
fn configured_running(patch: &Patch) -> BTreeMap<String, bool> {
    patch
        .nodes
        .iter()
        .filter(|node| node.kind == "seq3")
        .map(|node| {
            (
                node.id.clone(),
                node.params.get("running").copied().unwrap_or(1.0) == 1.0,
            )
        })
        .collect()
}

#[cfg(test)]
fn configured_holding(patch: &Patch) -> BTreeMap<String, bool> {
    patch
        .nodes
        .iter()
        .filter(|node| node.kind == "push")
        .map(|node| {
            (
                node.id.clone(),
                node.params.get("holding").copied().unwrap_or(0.0) == 1.0,
            )
        })
        .collect()
}

fn configured_saved_states(patch: &Patch) -> BTreeMap<String, SavedModuleState> {
    patch
        .nodes
        .iter()
        .filter_map(|node| SavedModuleState::from_node(node).map(|state| (node.id.clone(), state)))
        .collect()
}

fn document_content(document: &Document) -> Value {
    saved_content(
        &document.patch,
        document.layout.clone(),
        &configured_saved_states(&document.patch),
    )
}

// Normalize omitted state to its deterministic initial state only in this projection.
fn saved_content(
    patch: &Patch,
    layout: Value,
    states: &BTreeMap<String, SavedModuleState>,
) -> Value {
    let bits = states
        .iter()
        .filter_map(|(id, state)| match state {
            SavedModuleState::RandomValues { values, rng } => {
                Some((id, json!({"values":values,"rng":rng})))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut value = json!({"patch":patch,"layout":layout,"persistent_bits":bits});
    for node in value["patch"]["nodes"]
        .as_array_mut()
        .expect("serialized patch nodes")
    {
        if let Some(state) = node["id"].as_str().and_then(|id| states.get(id)) {
            let mut params: BTreeMap<String, f32> =
                serde_json::from_value(node["params"].clone()).expect("serialized params");
            state.write_params(&mut params);
            node["params"] = json!(params);
        }
    }
    value
}

pub struct AppliedDocument {
    pub patch: Patch,
    pub rack: Rack,
    pub revision: u64,
    pub state_id: u64,
}

/// Prepare every fallible part before committing the session. The caller only
/// installs the returned view/path/save baseline after this succeeds.
#[cfg(test)]
pub fn apply_document(
    shared: &openpatch::control::SharedSession,
    expected_revision: u64,
    document: &Document,
) -> Result<AppliedDocument, String> {
    apply_document_guarded(shared, expected_revision, document, |_| Ok(()))
}

pub fn apply_document_guarded(
    shared: &openpatch::control::SharedSession,
    expected_revision: u64,
    document: &Document,
    before_commit: impl FnOnce(&openpatch::control::Session) -> Result<(), String>,
) -> Result<AppliedDocument, String> {
    install_document_guarded(shared, expected_revision, document, false, before_commit)
}

pub fn new_document_guarded(
    shared: &openpatch::control::SharedSession,
    expected_revision: u64,
    document: &Document,
    before_commit: impl FnOnce(&openpatch::control::Session) -> Result<(), String>,
) -> Result<AppliedDocument, String> {
    install_document_guarded(shared, expected_revision, document, true, before_commit)
}

fn install_document_guarded(
    shared: &openpatch::control::SharedSession,
    expected_revision: u64,
    document: &Document,
    new_document: bool,
    before_commit: impl FnOnce(&openpatch::control::Session) -> Result<(), String>,
) -> Result<AppliedDocument, String> {
    let validated = Document::new(document.patch.clone(), document.layout.clone())?;
    let mut rack = Rack::default();
    rack.restore_layout(&validated.layout)?;
    rack.reconcile(&validated.patch, &openpatch_core::catalog());
    let mut live = shared.lock().map_err(|_| "Session lock poisoned")?;
    before_commit(&live)?;
    let revision = if new_document {
        live.new_document(validated.patch, Some(expected_revision))?
    } else {
        live.replace_patch(validated.patch, Some(expected_revision))?
    };
    Ok(AppliedDocument {
        patch: live.patch().clone(),
        rack,
        revision,
        state_id: live.state_id(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpatch::control::Session;

    #[test]
    fn run_projection_normalizes_defaults_and_unknown_state_never_claims_clean() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"seq","kind":"seq3"}
        ]}))
        .unwrap();
        let mut rack = Rack::default();
        rack.reconcile(&patch, &openpatch_core::catalog());
        let mut state = DocumentState::new(&patch, 1, &rack);
        let original = Document {
            patch: patch.clone(),
            layout: rack.layout(),
        };
        let running = |run| Some(BTreeMap::from([("seq".into(), run)]));
        assert!(!state.observe_running(&patch, 1, &rack, running(true)));
        assert!(!state.dirty());
        let mut explicit = original.clone();
        explicit.patch.nodes[0].params.insert("running".into(), 1.0);
        assert!(state.matches_saved(&explicit));
        assert!(state.observe_running(&patch, 1, &rack, running(false)));
        assert!(state.dirty());
        assert!(state.observe_running(&patch, 1, &rack, None));
        assert!(!state.observe_running(&patch, 1, &rack, None));
        state.mark_saved(&original);
        assert!(state.dirty());
        assert!(state.observe_running(&patch, 1, &rack, running(true)));
        assert!(!state.dirty());
        assert!(!state.observe_running(&patch, 1, &rack, running(true)));
    }

    #[test]
    fn hold_projection_normalizes_omitted_zero_and_tracks_live_flip_at_same_revision() {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"nodes":[{"id":"button","kind":"push"}]
        }))
        .unwrap();
        let mut rack = Rack::default();
        rack.reconcile(&patch, &openpatch_core::catalog());
        let mut state = DocumentState::new(&patch, 7, &rack);
        let original = Document {
            patch: patch.clone(),
            layout: rack.layout(),
        };
        let mut explicit_zero = original.clone();
        explicit_zero.patch.nodes[0]
            .params
            .insert("holding".into(), 0.0);
        assert!(state.matches_saved(&explicit_zero));
        let observed = |held| Some((BTreeMap::new(), BTreeMap::from([("button".into(), held)])));
        assert!(!state.observe_runtime(&patch, 7, &rack, observed(false)));
        assert!(!state.dirty());
        assert!(state.observe_runtime(&patch, 7, &rack, observed(true)));
        assert!(
            state.dirty(),
            "HOLD changed without a graph or layout revision"
        );
        assert!(!state.observe_runtime(&patch, 7, &rack, observed(true)));
        assert!(state.observe_runtime(&patch, 7, &rack, observed(false)));
        assert!(!state.dirty());
        assert!(state.observe_runtime(&patch, 7, &rack, None));
        assert!(
            state.dirty(),
            "unavailable live HOLD cannot be called clean"
        );
        assert!(state.observe_runtime(&patch, 7, &rack, observed(false)));
        assert!(!state.dirty());
    }

    #[test]
    fn newer_edits_remain_dirty_when_an_older_snapshot_finishes_saving() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(dir.path()).unwrap();
        let mut rack = Rack::default();
        let mut state = DocumentState::new(session.patch(), session.revision(), &rack);
        let captured = Document {
            patch: session.patch().clone(),
            layout: rack.layout(),
        };
        session
            .execute("add_module", json!({"id":"dc","kind":"constant"}))
            .unwrap();
        rack.reconcile(session.patch(), &openpatch_core::catalog());
        state.observe(session.patch(), session.revision(), &rack);
        state.mark_saved(&captured);
        assert!(state.dirty());
        session.execute("undo", json!({})).unwrap();
        rack.reconcile(session.patch(), &openpatch_core::catalog());
        state.observe(session.patch(), session.revision(), &rack);
        assert!(
            !state.dirty(),
            "undo restores saved content despite a newer revision"
        );
    }

    #[test]
    fn grouped_edits_after_save_and_view_only_changes_are_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(dir.path()).unwrap();
        session
            .execute("add_module", json!({"id":"dc","kind":"constant"}))
            .unwrap();
        session
            .execute(
                "set_parameter",
                json!({"id":"dc","parameter":"value","value":1,"history_group":"drag"}),
            )
            .unwrap();
        let mut rack = Rack::default();
        rack.reconcile(session.patch(), &openpatch_core::catalog());
        let mut state = DocumentState::new(session.patch(), session.revision(), &rack);
        let identity = session.state_id();
        session
            .execute(
                "set_parameter",
                json!({"id":"dc","parameter":"value","value":2,"history_group":"drag"}),
            )
            .unwrap();
        assert_eq!(identity, session.state_id());
        state.observe(session.patch(), session.revision(), &rack);
        assert!(state.dirty());
        let captured = Document {
            patch: session.patch().clone(),
            layout: rack.layout(),
        };
        state.mark_saved(&captured);
        assert!(!state.dirty());
        rack.set_zoom(0.75);
        assert!(state.observe(session.patch(), session.revision(), &rack));
        assert!(state.dirty());
        rack.reset_view();
        state.observe(session.patch(), session.revision(), &rack);
        assert!(!state.dirty());
        assert!(!state.observe(session.patch(), session.revision(), &rack));
    }
}

#[cfg(test)]
mod loading_tests {
    use super::*;
    use openpatch::control::Session;
    #[test]
    fn malformed_layout_and_stale_load_leave_session_and_history_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let shared = Session::new(dir.path()).unwrap().into_shared();
        let initial = shared.lock().unwrap().snapshot();
        let malformed = Document {
            patch: openpatch_core::demo_patch(),
            layout: json!({"version":1,"zoom":-1}),
        };
        assert!(apply_document(&shared, 0, &malformed).is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), initial);
        let mut rack = Rack::default();
        let mut patch = openpatch_core::demo_patch();
        patch.assign_missing_cable_ids().unwrap();
        rack.reconcile(&patch, &openpatch_core::catalog());
        let candidate = Document {
            patch,
            layout: rack.layout(),
        };
        shared
            .lock()
            .unwrap()
            .execute("add_module", json!({"id":"concurrent","kind":"constant"}))
            .unwrap();
        let newer = shared.lock().unwrap().snapshot();
        assert!(apply_document(&shared, 0, &candidate).is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), newer);
        let applied = apply_document(&shared, 1, &candidate).unwrap();
        assert_eq!(applied.revision, 2);
        assert_eq!(applied.rack.layout(), candidate.layout);
        assert_eq!(
            serde_json::to_value(applied.patch).unwrap(),
            serde_json::to_value(candidate.patch).unwrap()
        );
    }

    #[test]
    fn new_document_guard_rejection_keeps_graph_history_and_layout_then_success_clears_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::new(dir.path()).unwrap();
        session
            .execute("add_module", json!({"id":"old","kind":"constant"}))
            .unwrap();
        let shared = session.into_shared();
        let before = shared.lock().unwrap().snapshot();
        let candidate = crate::startup::factory_document();
        let original_layout = candidate.layout.clone();
        assert!(new_document_guarded(&shared, 1, &candidate, |_| Err(
            "unsaved runtime changes".into()
        ))
        .is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        assert_eq!(candidate.layout, original_layout);
        assert!(new_document_guarded(&shared, 0, &candidate, |_| Ok(())).is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        let applied = new_document_guarded(&shared, 1, &candidate, |_| Ok(())).unwrap();
        assert_eq!(applied.rack.layout(), candidate.layout);
        let live = shared.lock().unwrap();
        assert!(!live.can_undo() && !live.can_redo());
        assert_eq!(applied.revision, 2);
        assert_eq!(applied.state_id, live.state_id());
    }
}
