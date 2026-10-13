//! Content-based saved state, independent of revision/history identities.
//! Only refreshed when the graph or persisted rack layout changes.
use crate::{project::Document, rack::Rack};
use openpatch_core::Patch;
use serde_json::{json, Value};

pub struct DocumentState {
    current: Value,
    baseline: Option<Value>,
    observed: Option<(u64, u64)>,
    dirty: bool,
}
impl DocumentState {
    pub fn new(patch: &Patch, revision: u64, rack: &Rack) -> Self {
        let current = json!({"patch":patch,"layout":rack.layout()});
        Self {
            baseline: Some(current.clone()),
            current,
            observed: Some((revision, rack.layout_revision())),
            dirty: false,
        }
    }
    pub fn observe(&mut self, patch: &Patch, revision: u64, rack: &Rack) -> bool {
        let key = (revision, rack.layout_revision());
        if self.observed == Some(key) {
            return false;
        }
        self.observed = Some(key);
        let next = json!({"patch":patch,"layout":rack.layout()});
        let changed = self.current != next;
        self.current = next;
        self.dirty = self.baseline.as_ref() != Some(&self.current);
        changed
    }
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    pub fn mark_saved(&mut self, document: &Document) {
        self.baseline = Some(json!({"patch":document.patch,"layout":document.layout}));
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

pub struct AppliedDocument {
    pub patch: Patch,
    pub rack: Rack,
    pub revision: u64,
    pub state_id: u64,
}

/// Prepare every fallible part before committing the session. The caller only
/// installs the returned view/path/save baseline after this succeeds.
pub fn apply_document(
    shared: &openpatch::control::SharedSession,
    expected_revision: u64,
    document: &Document,
) -> Result<AppliedDocument, String> {
    let validated = Document::new(document.patch.clone(), document.layout.clone())?;
    let mut rack = Rack::default();
    rack.restore_layout(&validated.layout)?;
    rack.reconcile(&validated.patch, &openpatch_core::catalog());
    let mut live = shared.lock().map_err(|_| "Session lock poisoned")?;
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
