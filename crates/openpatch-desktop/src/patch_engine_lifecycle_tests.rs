use super::*;
use crate::patch_engine_ui::{DocumentFence, GeneratedDocument};

fn mark_generated(app: &mut OpenPatch) {
    app.capture_document();
    let fence = DocumentFence::capture(&app.shared.lock().unwrap());
    app.patch_engine_last = Some(GeneratedDocument::capture(&app.shared, fence).unwrap());
}

#[test]
fn unchanged_generated_document_regenerates_without_prompt_with_fresh_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.replace_now(openpatch_core::demo_patch(), "Generated");
    mark_generated(&mut app);
    let old_generation = app
        .shared
        .lock()
        .unwrap()
        .project_save_baseline()
        .generation();
    app.prepare_patch_engine("Next song");
    assert!(app.confirm_discard.is_none());
    assert!(matches!(app.patch_engine_wait, Some(Some(_))));
    assert_eq!(app.title, "Next song");
    assert!(app.patch.nodes.is_empty());
    assert_eq!(
        app.shared
            .lock()
            .unwrap()
            .project_save_baseline()
            .generation(),
        old_generation + 1
    );
}

#[test]
fn manual_edit_to_generated_song_requires_discard_and_cancel_keeps_the_edit() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.replace_now(openpatch_core::demo_patch(), "Generated");
    mark_generated(&mut app);
    edit(&mut app, 321.0);
    let before = app.patch.reference_view();
    app.prepare_patch_engine("Generated");
    assert!(matches!(
        app.confirm_discard,
        Some(PendingAction::PatchEngine(_))
    ));
    app.confirm_discard = None;
    app.service_patch_engine();
    assert!(app.patch_engine_wait.is_none());
    assert_eq!(app.patch.reference_view(), before);
}

#[test]
fn cancel_same_title_empty_document_never_yields_a_build_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.replace_now(Patch::new_v2(), "Same title");
    let revision = app.revision;
    app.prepare_patch_engine("Same title");
    assert!(matches!(
        app.confirm_discard,
        Some(PendingAction::PatchEngine(_))
    ));
    assert!(matches!(app.patch_engine_wait, Some(None)));
    app.confirm_discard = None;
    app.service_patch_engine();
    assert!(app.patch_engine_wait.is_none());
    assert_eq!(app.revision, revision);
}

#[test]
fn explicit_discard_produces_a_handoff_only_for_the_successful_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    edit(&mut app, 321.0);
    app.prepare_patch_engine("New song");
    let action = app.confirm_discard.take().unwrap();
    app.perform_action(action);
    assert!(matches!(app.patch_engine_wait, Some(Some(_))));
    assert!(app.active_file.is_none());
    assert_eq!(app.title, "New song");
    assert!(app.dirty());
}

#[test]
fn replacement_and_save_revoke_generated_scratch_authority() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.replace_now(openpatch_core::demo_patch(), "Generated");
    mark_generated(&mut app);
    app.replace_now(openpatch_core::demo_patch(), "Generated");
    assert!(app.patch_engine_last.is_none());
    mark_generated(&mut app);
    app.save_document("generated.openpatch", true);
    edit(&mut app, 321.0);
    app.prepare_patch_engine("Next song");
    assert!(matches!(
        app.confirm_discard,
        Some(PendingAction::PatchEngine(_))
    ));
}
