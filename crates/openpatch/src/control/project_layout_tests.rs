//! Host view publication stays independent from graph/history/audio authority.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Live {
    calls: AtomicUsize,
}
impl LiveModuleEvents for Live {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Err("unexpected trigger".into())
    }
    fn snapshot(&self) -> Value {
        self.calls.fetch_add(1, Ordering::Relaxed);
        json!({"coherent":false})
    }
    fn sync_patch(&self, _: &Patch, _: u64) {
        self.calls.fetch_add(1, Ordering::Relaxed);
    }
}

fn session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute("add_module", json!({"id":"dc","kind":"constant"}))
        .unwrap();
    s
}
fn positioned(s: &Session) -> ProjectLayout {
    let mut layout = s.project_document().layout.clone();
    for (i, node) in s.project_document().graph.nodes.iter().enumerate() {
        layout
            .positions
            .entry(node.id().to_owned())
            .or_insert([i as f32 * 100.0, 0.0]);
    }
    layout
}
fn publish(s: &mut Session, layout: ProjectLayout, intent: LayoutIntent) -> Result<Value, String> {
    s.publish_host_layout(
        s.revision(),
        s.state_id(),
        s.layout_generation(),
        layout,
        intent,
    )
}
fn attach(s: &mut Session) {
    s.attach_layout_host().unwrap();
    let layout = positioned(s);
    publish(s, layout, LayoutIntent::Placement).unwrap();
}

#[test]
fn camera_persists_without_history_redo_graph_or_audio_changes() {
    let mut s = session();
    attach(&mut s);
    s.execute(
        "set_parameter",
        json!({"id":"dc","parameter":"value","value":3.0}),
    )
    .unwrap();
    assert!(!s.layout_pending());
    s.execute("undo", json!({})).unwrap();
    assert!(s.can_redo());
    let live = Arc::new(Live::default());
    s.set_live_module_events(Some(live.clone())).unwrap();
    live.calls.store(0, Ordering::Relaxed);
    let before = s.snapshot();
    let ticket = s.prepared_wave_banks();
    let generation = s.layout_generation();
    let disposition = s.execution_disposition();
    let mut layout = s.project_document().layout.clone();
    layout.pan = [240.0, -135.0];
    layout.zoom = 1.75;
    let receipt = publish(&mut s, layout.clone(), LayoutIntent::Camera).unwrap();
    assert_eq!(
        s.snapshot(),
        before,
        "legacy native snapshots keep their exact shape"
    );
    assert_eq!(s.project_document().layout, layout);
    assert_eq!(receipt["layout"], serde_json::to_value(&layout).unwrap());
    assert_eq!(receipt["layout_generation"], generation + 1);
    assert_eq!(receipt["revision"], s.revision());
    assert_eq!(receipt["state_id"], s.state_id());
    assert_eq!(s.execution_disposition(), disposition);
    assert!(Arc::ptr_eq(&s.prepared_wave_banks(), &ticket));
    assert_eq!(live.calls.load(Ordering::Relaxed), 0);
    let captured = s.capture_project_document(Some(s.revision())).unwrap();
    assert_eq!(captured.layout_generation, generation + 1);
    assert_eq!(captured.document.layout, layout);
}

#[test]
fn stale_and_invalid_publications_leave_layout_history_and_generation_untouched() {
    let mut s = session();
    attach(&mut s);
    let before = s.snapshot();
    let layout = s.project_document().layout.clone();
    let generation = s.layout_generation();
    let (revision, state) = (s.revision(), s.state_id());
    for (rev, state, generation) in [
        (revision - 1, state, generation),
        (revision, state + 1, generation),
        (revision, state, generation - 1),
    ] {
        assert!(s
            .publish_host_layout(rev, state, generation, layout.clone(), LayoutIntent::Camera)
            .is_err());
    }
    let mut changed_positions = layout.clone();
    changed_positions
        .positions
        .insert("dc".into(), [900.0, 0.0]);
    assert!(publish(&mut s, changed_positions, LayoutIntent::Camera).is_err());
    let mut invalid = layout.clone();
    invalid.zoom = f32::NAN;
    assert!(publish(&mut s, invalid, LayoutIntent::Placement).is_err());
    let mut incomplete = layout.clone();
    incomplete.positions.remove("dc");
    assert!(publish(&mut s, incomplete, LayoutIntent::Placement).is_err());
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.project_document().layout, layout);
    assert_eq!(s.layout_generation(), generation);
    assert!(!s.layout_pending());
}

#[test]
fn pending_placement_retries_capture_and_finalizes_without_extra_undo_step() {
    let mut s = session();
    assert!(
        !s.layout_pending(),
        "headless sessions never await host placement"
    );
    assert!(s.capture_project_document(None).is_ok());
    s.attach_layout_host().unwrap();
    let attached_generation = s.layout_generation();
    s.attach_layout_host().unwrap();
    assert_eq!(
        s.layout_generation(),
        attached_generation,
        "attachment is idempotent"
    );
    assert!(s
        .capture_project_document(None)
        .unwrap_err()
        .contains("placement is pending"));
    let camera = s.project_document().layout.clone();
    assert!(publish(&mut s, camera, LayoutIntent::Camera)
        .unwrap_err()
        .contains("pending"));
    let placement = positioned(&s);
    publish(&mut s, placement, LayoutIntent::Placement).unwrap();
    s.execute("add_module", json!({"id":"second","kind":"constant"}))
        .unwrap();
    assert!(s.layout_pending());
    s.execute(
        "set_parameter",
        json!({"id":"dc","parameter":"value","value":4.0}),
    )
    .unwrap();
    assert!(
        s.layout_pending(),
        "intervening commits cannot clear unfinished placement"
    );
    let before = s.snapshot();
    let placement = positioned(&s);
    publish(&mut s, placement, LayoutIntent::Placement).unwrap();
    assert_eq!(s.snapshot(), before);
    assert!(!s.layout_pending());
    assert!(s.capture_project_document(None).is_ok());
    s.execute("undo", json!({})).unwrap();
    assert_eq!(
        s.patch().nodes.len(),
        2,
        "placement must not insert an undo step"
    );
    assert!(
        s.layout_pending(),
        "unfinalized geometry in an earlier state must be repaired before capture"
    );
    let placement = positioned(&s);
    publish(&mut s, placement, LayoutIntent::Placement).unwrap();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.patch().nodes.len(), 1);
}

#[test]
fn geometry_history_restores_positions_and_keeps_the_current_camera() {
    let mut s = session();
    attach(&mut s);
    let original = s.project_document().layout.positions.clone();
    let mut moved = s.project_document().layout.clone();
    moved.positions.insert("dc".into(), [500.0, 380.0]);
    s.record_project_layout(moved.clone(), s.revision())
        .unwrap();
    let mut camera = s.project_document().layout.clone();
    camera.pan = [-300.0, 140.0];
    camera.zoom = 2.0;
    publish(&mut s, camera, LayoutIntent::Camera).unwrap();
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.project_document().layout.positions, original);
    assert_eq!(s.project_document().layout.pan, [-300.0, 140.0]);
    assert_eq!(s.project_document().layout.zoom, 2.0);
    assert!(!s.layout_pending());
    let mut camera = s.project_document().layout.clone();
    camera.zoom = 1.25;
    publish(&mut s, camera, LayoutIntent::Camera).unwrap();
    s.execute("redo", json!({})).unwrap();
    assert_eq!(s.project_document().layout.positions, moved.positions);
    assert_eq!(s.project_document().layout.pan, [-300.0, 140.0]);
    assert_eq!(s.project_document().layout.zoom, 1.25);
}

#[test]
fn prepared_replacement_installs_geometry_atomically_and_resets_new_document_history() {
    let mut s = session();
    attach(&mut s);
    let mut document = s.project_document().clone();
    document.layout.pan = [125.0, -40.0];
    document.layout.positions.insert("dc".into(), [700.0, 0.0]);
    s.replace_prepared_project_document(document.clone(), Some(s.revision()))
        .unwrap();
    assert!(!s.layout_pending());
    assert_eq!(
        s.capture_project_document(None).unwrap().document.layout,
        document.layout
    );
    let before = s.snapshot();
    let generation = s.layout_generation();
    let mut incomplete = document.clone();
    incomplete.layout.positions.clear();
    assert!(s
        .new_prepared_project_document(incomplete, Some(s.revision()))
        .is_err());
    assert_eq!(s.snapshot(), before);
    assert_eq!(s.layout_generation(), generation);
    s.new_prepared_project_document(document, Some(s.revision()))
        .unwrap();
    assert!(!s.layout_pending());
    assert!(!s.can_undo() && !s.can_redo());
}

#[test]
fn imported_document_inspection_and_save_receipts_include_layout_generation() {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.replace_project_document(super::document_tests::imported(), None)
        .unwrap();
    attach(&mut s);
    assert_eq!(s.snapshot()["layout_generation"], s.layout_generation());
    assert_eq!(s.snapshot()["layout_pending"], false);
    let inspect = s.execute("inspect_document", json!({})).unwrap();
    assert_eq!(inspect["layout_generation"], s.layout_generation());
    let captured = s.capture_project_document(None).unwrap();
    let filename = format!(
        "openpatch-layout-test-{}-{}.json",
        std::process::id(),
        s.layout_generation()
    );
    let path = std::env::temp_dir().join(&filename);
    let _ = fs::remove_file(&path);
    let saved = document::write_document_snapshot(&path, &filename, captured).unwrap();
    assert_eq!(saved["layout_generation"], s.layout_generation());
    fs::remove_file(path).unwrap();
}

#[test]
fn console_width_changes_and_legacy_move_receipts_fence_capture_until_settled() {
    let mut s = session();
    s.execute(
        "add_module",
        json!({"id":"console","kind":"studio_mixer","params":{"channels":2}}),
    )
    .unwrap();
    attach(&mut s);
    s.execute(
        "set_parameter",
        json!({"id":"console","parameter":"channels","value":4}),
    )
    .unwrap();
    assert!(
        s.layout_pending(),
        "channel count changes the actual console width"
    );
    assert!(s.capture_project_document(None).is_err());
    let layout = positioned(&s);
    publish(&mut s, layout, LayoutIntent::Placement).unwrap();
    assert!(s.capture_project_document(None).is_ok());
    s.record_layout_checkpoint(s.revision()).unwrap();
    assert!(
        s.layout_pending(),
        "an exact legacy move receipt still awaits its positions"
    );
    s.execute(
        "set_parameter",
        json!({"id":"dc","parameter":"value","value":2}),
    )
    .unwrap();
    assert!(
        s.layout_pending(),
        "a later parameter commit cannot certify older unplaced geometry"
    );
    assert!(s.capture_project_document(None).is_err());
    let mut layout = positioned(&s);
    layout.positions.insert("dc".into(), [90., 420.]);
    publish(&mut s, layout, LayoutIntent::Placement).unwrap();
    assert_eq!(
        s.capture_project_document(None)
            .unwrap()
            .document
            .layout
            .positions["dc"],
        [90., 420.]
    );
}
