//! Content-based saved state, independent of revision/history identities.
//! Refreshed when graph, persisted rack layout, or effective persistent module state changes.
use crate::{project::Document, rack::Rack};
use openpatch_core::Patch;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct DocumentState {
    current: Value,
    baseline: Option<Value>,
    observed: Option<(u64, u64)>,
    dirty: bool,
    running: BTreeMap<String, bool>,
    holding: BTreeMap<String, bool>,
    known: bool,
}
impl DocumentState {
    pub fn new(patch: &Patch, revision: u64, rack: &Rack) -> Self {
        let running = configured_running(patch);
        let holding = configured_holding(patch);
        let current = content(patch, rack.layout(), &running, &holding);
        Self {
            baseline: Some(current.clone()),
            current,
            observed: Some((revision, rack.layout_revision())),
            dirty: false,
            running,
            holding,
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
    pub fn observe_runtime(
        &mut self,
        patch: &Patch,
        revision: u64,
        rack: &Rack,
        states: Option<(BTreeMap<String, bool>, BTreeMap<String, bool>)>,
    ) -> bool {
        let Some((running, holding)) = states else {
            let changed = self.known;
            self.known = false;
            self.observed = None;
            return changed;
        };
        let key = (revision, rack.layout_revision());
        if self.known
            && self.observed == Some(key)
            && self.running == running
            && self.holding == holding
        {
            return false;
        }
        let was_unknown = !self.known;
        self.known = true;
        self.observed = Some(key);
        let next = content(patch, rack.layout(), &running, &holding);
        self.running = running;
        self.holding = holding;
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

fn document_content(document: &Document) -> Value {
    content(
        &document.patch,
        document.layout.clone(),
        &configured_running(&document.patch),
        &configured_holding(&document.patch),
    )
}

// Omitted persistent flags and their explicit defaults are equal saved content.
// Normalize only the serialized projection; never edit the live patch/history.
fn content(
    patch: &Patch,
    layout: Value,
    running: &BTreeMap<String, bool>,
    holding: &BTreeMap<String, bool>,
) -> Value {
    let mut value = json!({"patch":patch,"layout":layout});
    for node in value["patch"]["nodes"]
        .as_array_mut()
        .expect("serialized patch nodes")
    {
        let (parameter, values) = match node["kind"].as_str() {
            Some("seq3") => ("running", running),
            Some("push") => ("holding", holding),
            _ => continue,
        };
        if let Some(value) = node["id"].as_str().and_then(|id| values.get(id)) {
            node["params"][parameter] = json!(if *value { 1.0 } else { 0.0 });
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
    let validated = Document::new(document.patch.clone(), document.layout.clone())?;
    let mut rack = Rack::default();
    rack.restore_layout(&validated.layout)?;
    rack.reconcile(&validated.patch, &openpatch_core::catalog());
    let mut live = shared.lock().map_err(|_| "Session lock poisoned")?;
    before_commit(&live)?;
    let revision = live.replace_patch(validated.patch.clone(), Some(expected_revision))?;
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
}
