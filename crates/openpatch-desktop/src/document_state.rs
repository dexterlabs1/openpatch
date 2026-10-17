//! Content-based saved state, independent of revision/history identities.
//! Refreshed from authored graph/layout and durable persistent module state.
use crate::{project::Document, rack::Rack};
use openpatch::control::{
    configured_document_states, saved_content, ProjectSaveCheckpoint, SharedProjectSave,
};
use openpatch::project_document::ProjectDocument;
use openpatch_core::{Patch, SavedModuleState};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;

pub struct DocumentState {
    current: Value,
    baseline: SharedProjectSave,
    observed: Option<(u64, u64)>,
    dirty: bool,
    known: bool,
}
impl DocumentState {
    pub fn new(project: &ProjectDocument, revision: u64, rack: &Rack) -> Self {
        let saved_states = configured_document_states(project);
        let current = saved_content(project, rack.layout(), &saved_states);
        let baseline = SharedProjectSave::default();
        baseline
            .mark_initial(current.clone())
            .expect("fresh save baseline");
        Self {
            baseline,
            current,
            observed: Some((revision, rack.layout_revision())),
            dirty: false,
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
        let document =
            Document::new(patch.clone(), rack.layout()).expect("valid native test document");
        self.observe_saved_runtime(document.project(), revision, rack, states)
    }
    #[cfg(test)]
    pub fn observe_saved_runtime(
        &mut self,
        project: &ProjectDocument,
        revision: u64,
        rack: &Rack,
        states: Option<BTreeMap<String, SavedModuleState>>,
    ) -> bool {
        let Some(saved_states) = states else {
            return self.observe_canonical(None);
        };
        let next = saved_content(project, rack.layout(), &saved_states);
        self.observed = Some((revision, rack.layout_revision()));
        self.observe_canonical(Some(next))
    }

    /// Attach the Session authority without implicitly marking its document saved.
    /// Startup/new-template callers explicitly establish their initial baseline.
    pub fn attach_baseline(&mut self, baseline: SharedProjectSave) {
        self.baseline = baseline;
        self.dirty = !self.baseline.matches_content(&self.current);
    }

    pub fn observe_canonical(&mut self, content: Option<Value>) -> bool {
        let Some(next) = content else {
            let changed = self.known;
            self.known = false;
            self.observed = None;
            return changed;
        };
        let dirty = !self.baseline.matches_content(&next);
        let changed = !self.known || self.current != next || self.dirty != dirty;
        self.current = next;
        self.known = true;
        self.dirty = dirty;
        changed
    }

    pub fn dirty(&self) -> bool {
        !self.known || !self.baseline.matches_content(&self.current)
    }
    #[cfg(test)]
    pub fn matches_saved(&self, document: &Document) -> bool {
        self.baseline.matches_content(&document_content(document))
    }
    pub fn mark_saved(&mut self, document: &Document) {
        let generation = self.baseline.status_from_content(None)["generation"]
            .as_u64()
            .unwrap_or(0);
        let _ = self.baseline.register(
            &ProjectSaveCheckpoint {
                generation,
                revision: self.observed.map_or(0, |(revision, _)| revision),
                frame: None,
                edit_content: document_content(document),
            },
            None,
        );
        self.dirty = !self.baseline.matches_content(&self.current);
    }
    pub fn mark_unsaved(&mut self) {
        let _ = self.baseline.mark_unsaved();
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
        document.project(),
        document.layout().clone(),
        &configured_saved_states(document.patch()),
    )
}

pub struct AppliedDocument {
    pub project: ProjectDocument,
    pub patch: Patch,
    pub rack: Rack,
    pub revision: u64,
    pub state_id: u64,
    pub layout_generation: u64,
    pub document_generation: u64,
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
    let mut rack = Rack::default();
    rack.restore_layout(document.layout())?;
    rack.reconcile_document(
        document.patch(),
        document.project(),
        &openpatch_core::catalog(),
    );
    let document = document.clone().with_layout(rack.layout())?;
    let mut live = shared.lock().map_err(|_| "Session lock poisoned")?;
    before_commit(&live)?;
    let revision = if new_document {
        live.new_prepared_project_document(document.project().clone(), Some(expected_revision))?
    } else {
        live.replace_prepared_project_document(document.project().clone(), Some(expected_revision))?
    };
    Ok(AppliedDocument {
        project: live.project_document().clone(),
        patch: live.patch().clone(),
        rack,
        revision,
        state_id: live.state_id(),
        layout_generation: live.layout_generation(),
        document_generation: live.project_save_baseline().generation(),
    })
}

#[cfg(test)]
mod tests {
    include!("document_state_tests.rs");
    use super::*;
    use openpatch::control::Session;

    #[test]
    fn opaque_payload_assets_and_layout_dirty_even_when_native_carrier_and_revision_match() {
        use crate::project::test_source_document;
        let original = test_source_document(json!({"value":1}), &[1, 2, 3]);
        let mut rack = Rack::default();
        rack.restore_layout(original.layout()).unwrap();
        rack.reconcile_document(
            original.patch(),
            original.project(),
            &openpatch_core::catalog(),
        );
        let mut state = DocumentState::new(original.project(), 7, &rack);
        state.mark_saved(&original);
        assert!(!state.dirty());
        for changed in [
            test_source_document(json!({"value":2}), &[1, 2, 3]),
            test_source_document(json!({"value":1}), &[1, 2, 4]),
        ] {
            assert_eq!(
                original.patch().reference_view(),
                changed.patch().reference_view()
            );
            assert!(!state.matches_saved(&changed));
            assert!(state.observe_saved_runtime(
                changed.project(),
                7,
                &rack,
                Some(BTreeMap::new())
            ));
            assert!(
                state.dirty(),
                "opaque changes cannot be hidden by an unchanged native carrier"
            );
            assert!(state.observe_saved_runtime(
                original.project(),
                7,
                &rack,
                Some(BTreeMap::new())
            ));
            assert!(!state.dirty());
        }
        let mut layout = original.layout().clone();
        layout["positions"]["unknown"] = json!([45.0, 380.0]);
        let moved = original.clone().with_layout(layout).unwrap();
        rack.restore_layout(moved.layout()).unwrap();
        assert!(state.observe_saved_runtime(moved.project(), 7, &rack, Some(BTreeMap::new())));
        assert!(state.dirty());
        state.mark_saved(&moved);
        assert!(!state.dirty());
        assert!(state.matches_saved(&moved));
        state.mark_saved(&original);
        assert!(
            state.dirty(),
            "an older completed save cannot clean a newer source layout"
        );
    }

    #[test]
    fn run_projection_normalizes_defaults_and_unknown_state_never_claims_clean() {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"seq","kind":"seq3"}
        ]}))
        .unwrap();
        let mut rack = Rack::default();
        rack.reconcile(&patch, &openpatch_core::catalog());
        let original = Document::new(patch.clone(), rack.layout()).unwrap();
        let mut state = DocumentState::new(original.project(), 1, &rack);
        let running = |run| Some(BTreeMap::from([("seq".into(), run)]));
        assert!(!state.observe_running(&patch, 1, &rack, running(true)));
        assert!(!state.dirty());
        let mut explicit = original.patch().clone();
        explicit.nodes[0].params.insert("running".into(), 1.0);
        let explicit = Document::new(explicit, original.layout().clone()).unwrap();
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
        let original = Document::new(patch.clone(), rack.layout()).unwrap();
        let mut state = DocumentState::new(original.project(), 7, &rack);
        let mut explicit_zero = original.patch().clone();
        explicit_zero.nodes[0].params.insert("holding".into(), 0.0);
        let explicit_zero = Document::new(explicit_zero, original.layout().clone()).unwrap();
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
        let mut state = DocumentState::new(session.project_document(), session.revision(), &rack);
        let captured = Document::new(session.patch().clone(), rack.layout()).unwrap();
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
        let mut state = DocumentState::new(session.project_document(), session.revision(), &rack);
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
        let captured = Document::new(session.patch().clone(), rack.layout()).unwrap();
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
    fn full_document_apply_preserves_opaque_content_and_resets_playback_permission_atomically() {
        use crate::project::test_source_document;
        use openpatch::project_document::{ExecutionDisposition, PlaybackMode};
        let dir = tempfile::tempdir().unwrap();
        let shared = Session::new(dir.path()).unwrap().into_shared();
        let original = test_source_document(json!({"opaque":1}), &[2, 4, 6]);
        let applied = apply_document(&shared, 0, &original).unwrap();
        assert_eq!(applied.rack.layout(), *original.layout());
        {
            let mut live = shared.lock().unwrap();
            assert_eq!(
                live.project_document().encode().unwrap(),
                original.encode().unwrap()
            );
            assert_eq!(
                live.execution_disposition(),
                ExecutionDisposition::SuspendedUnresolved
            );
            assert_eq!(live.playback_mode(), PlaybackMode::Exact);
            assert_eq!(
                live.capture_project_document(Some(1)).unwrap().state_source,
                "suspended_persisted"
            );
            live.set_document_playback(PlaybackMode::Partial, Some(1))
                .unwrap();
        }
        let revised = test_source_document(json!({"opaque":2}), &[2, 4, 7]);
        let before = shared.lock().unwrap().snapshot();
        assert!(
            apply_document_guarded(&shared, 2, &revised, |_| Err("discard rejected".into()))
                .is_err()
        );
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        assert!(apply_document(&shared, 1, &revised).is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        let next = apply_document(&shared, 2, &revised).unwrap();
        assert_ne!(next.state_id, applied.state_id);
        let mut live = shared.lock().unwrap();
        assert_eq!(
            live.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        assert_eq!(
            live.project_document().encode().unwrap(),
            revised.encode().unwrap()
        );
        live.execute("undo", json!({})).unwrap();
        assert_eq!(live.playback_mode(), PlaybackMode::Partial);
        assert_eq!(
            live.project_document().encode().unwrap(),
            original.encode().unwrap()
        );
        live.execute("redo", json!({})).unwrap();
        assert_eq!(live.playback_mode(), PlaybackMode::Exact);
        assert_eq!(
            live.project_document().encode().unwrap(),
            revised.encode().unwrap()
        );
        let revision = live.revision();
        drop(live);
        new_document_guarded(&shared, revision, &original, |_| Ok(())).unwrap();
        let live = shared.lock().unwrap();
        assert!(!live.can_undo() && !live.can_redo());
        assert_eq!(
            live.project_document().encode().unwrap(),
            original.encode().unwrap()
        );
        assert_eq!(
            live.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
    }

    #[test]
    fn malformed_layout_and_stale_load_leave_session_and_history_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let shared = Session::new(dir.path()).unwrap().into_shared();
        let initial = shared.lock().unwrap().snapshot();
        assert!(
            Document::new(openpatch_core::demo_patch(), json!({"version":1,"zoom":-1})).is_err()
        );
        assert_eq!(shared.lock().unwrap().snapshot(), initial);
        let mut rack = Rack::default();
        let mut patch = openpatch_core::demo_patch();
        patch.assign_missing_cable_ids().unwrap();
        rack.reconcile(&patch, &openpatch_core::catalog());
        let candidate = Document::new(patch, rack.layout()).unwrap();
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
        assert_eq!(applied.rack.layout(), *candidate.layout());
        assert_eq!(
            serde_json::to_value(applied.patch).unwrap(),
            serde_json::to_value(candidate.patch()).unwrap()
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
        let original_layout = candidate.layout().clone();
        assert!(new_document_guarded(&shared, 1, &candidate, |_| Err(
            "unsaved runtime changes".into()
        ))
        .is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        assert_eq!(*candidate.layout(), original_layout);
        assert!(new_document_guarded(&shared, 0, &candidate, |_| Ok(())).is_err());
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        let applied = new_document_guarded(&shared, 1, &candidate, |_| Ok(())).unwrap();
        assert_eq!(applied.rack.layout(), *candidate.layout());
        let live = shared.lock().unwrap();
        assert!(!live.can_undo() && !live.can_redo());
        assert_eq!(applied.revision, 2);
        assert_eq!(applied.state_id, live.state_id());
    }
}
