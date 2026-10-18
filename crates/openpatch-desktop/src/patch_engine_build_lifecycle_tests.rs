use super::*;

fn add(id: &str) -> Value {
    json!({"tool":"add_module","arguments":{"id":id,"kind":"constant","params":{"value":1.0}}})
}

#[test]
fn stale_build_shell_never_adopts_an_external_revision() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let mut fence = DocumentFence::capture(&session.lock().unwrap());
    execute_shared(
        &session,
        "apply_edits",
        json!({"expected_revision":fence.revision,"edits":[add("external")]}),
    )
    .unwrap();
    assert!(apply_build_edits(&session, &mut fence, &[add("generated")]).is_err());
    assert_eq!(session.lock().unwrap().patch().nodes.len(), 1);
    assert_eq!(session.lock().unwrap().patch().nodes[0].id, "external");
}

#[test]
fn build_chunks_advance_only_from_their_own_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let mut fence = DocumentFence::capture(&session.lock().unwrap());
    apply_build_edits(&session, &mut fence, &[add("first")]).unwrap();
    execute_shared(
        &session,
        "apply_edits",
        json!({"expected_revision":fence.revision,"edits":[add("external")]}),
    )
    .unwrap();
    assert!(apply_build_edits(&session, &mut fence, &[add("second")]).is_err());
    assert_eq!(session.lock().unwrap().patch().nodes.len(), 2);
}

#[test]
fn fresh_document_invalidates_build_identity_even_with_identical_content() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let fence = DocumentFence::capture(&session.lock().unwrap());
    let mut live = session.lock().unwrap();
    live.new_document(openpatch_core::Patch::new_v2(), Some(fence.revision))
        .unwrap();
    assert!(!fence.matches(&live));
}

#[test]
fn construction_start_after_shell_rejects_intervening_document_edit() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let mut fence = DocumentFence::capture(&session.lock().unwrap());
    apply_build_edits(&session, &mut fence, &[add("shell")]).unwrap();
    execute_shared(
        &session,
        "apply_edits",
        json!({"expected_revision":fence.revision,"edits":[add("external")]}),
    )
    .unwrap();
    let steps = [json!({"label":"Generated layer","edits":[add("layer")]})];
    assert!(start_build_construction(&session, fence, 100, &steps, "Test").is_err());
    assert_eq!(
        session.lock().unwrap().construction_snapshot()["status"],
        "idle"
    );
    assert_eq!(session.lock().unwrap().patch().nodes.len(), 2);
}

#[test]
fn layout_only_switch_rejects_actual_shell_and_construction_without_history_changes() {
    for construction in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path()).unwrap().into_shared();
        let mut fence = DocumentFence::capture(&session.lock().unwrap());
        session.lock().unwrap().attach_layout_host().unwrap();
        let before = session.lock().unwrap().snapshot();
        let result = if construction {
            let steps = [json!({"label":"Generated layer","edits":[add("layer")]})];
            start_build_construction(&session, fence, 100, &steps, "Test").map(|_| ())
        } else {
            apply_build_edits(&session, &mut fence, &[add("generated")])
        };
        assert!(result.is_err());
        assert_eq!(session.lock().unwrap().snapshot(), before);
        assert_eq!(
            session.lock().unwrap().construction_snapshot()["status"],
            "idle"
        );
    }
}

#[test]
fn same_content_new_document_rejects_actual_shell_and_construction() {
    for construction in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::new(dir.path()).unwrap().into_shared();
        let mut fence = DocumentFence::capture(&session.lock().unwrap());
        {
            let mut live = session.lock().unwrap();
            live.new_document(openpatch_core::Patch::new_v2(), Some(fence.revision))
                .unwrap();
            // Isolate document identity from revision/layout checks: even a
            // caller carrying current counters cannot reuse the old generation.
            fence.revision = live.revision();
            fence.layout_generation = live.layout_generation();
            fence.layout_pending = live.layout_pending();
        }
        let before = session.lock().unwrap().snapshot();
        let result = if construction {
            let steps = [json!({"label":"Generated layer","edits":[add("layer")]})];
            start_build_construction(&session, fence, 100, &steps, "Test").map(|_| ())
        } else {
            apply_build_edits(&session, &mut fence, &[add("generated")])
        };
        assert!(result.is_err());
        assert_eq!(session.lock().unwrap().snapshot(), before);
        assert_eq!(
            session.lock().unwrap().construction_snapshot()["status"],
            "idle"
        );
    }
}

#[test]
fn automatic_pending_placement_is_allowed_and_next_fence_is_from_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(dir.path()).unwrap().into_shared();
    let mut fence = {
        let mut live = session.lock().unwrap();
        live.attach_layout_host().unwrap();
        let fence = DocumentFence::capture(&live);
        let layout = live.project_document().layout.clone();
        let state = live.state_id();
        live.publish_host_layout(
            fence.revision,
            state,
            fence.layout_generation,
            layout,
            openpatch::control::LayoutIntent::Placement,
        )
        .unwrap();
        fence
    };
    apply_build_edits(&session, &mut fence, &[add("generated")]).unwrap();
    assert_eq!(fence, DocumentFence::capture(&session.lock().unwrap()));
    assert!(fence.layout_pending);
}
