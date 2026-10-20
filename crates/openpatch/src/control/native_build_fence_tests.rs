use super::transport_authorization_tests::{construction_request, Boundary, Pause};
use super::*;

#[test]
fn native_build_fence_rechecks_layout_after_construction_preflight() {
    let directory = std::env::temp_dir().join(format!(
        "openpatch-native-fence-layout-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let session = Session::new(&directory).unwrap().into_shared();
    let fence = DocumentExecutionFence::capture(&session.lock().unwrap());
    let mut pause = Pause::install(&session, Boundary::ConstructionRegistration);
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        execute_shared_fenced(
            &worker_session,
            "start_construction",
            construction_request(fence.revision),
            fence,
        )
    });
    pause.wait();
    session.lock().unwrap().attach_layout_host().unwrap();
    let before = session.lock().unwrap().snapshot();
    pause.release();
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("document changed"));
    assert_eq!(session.lock().unwrap().snapshot(), before);
    assert_eq!(
        session.lock().unwrap().construction_snapshot()["status"],
        "idle"
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn native_build_fence_rechecks_generation_after_construction_preflight() {
    let directory = std::env::temp_dir().join(format!(
        "openpatch-native-fence-document-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let session = Session::new(&directory).unwrap().into_shared();
    let fence = DocumentExecutionFence::capture(&session.lock().unwrap());
    let mut pause = Pause::install(&session, Boundary::ConstructionRegistration);
    let worker_session = session.clone();
    let worker = std::thread::spawn(move || {
        execute_shared_fenced(
            &worker_session,
            "start_construction",
            construction_request(fence.revision),
            fence,
        )
    });
    pause.wait();
    {
        let mut live = session.lock().unwrap();
        live.new_document(Patch::new_v2(), Some(fence.revision))
            .unwrap();
    }
    let before = session.lock().unwrap().snapshot();
    pause.release();
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("document changed"));
    assert_eq!(session.lock().unwrap().snapshot(), before);
    assert_eq!(
        session.lock().unwrap().construction_snapshot()["status"],
        "idle"
    );
    let _ = std::fs::remove_dir_all(directory);
}

fn placed_session(name: &str) -> (std::path::PathBuf, SharedSession) {
    let directory = std::env::temp_dir().join(format!(
        "openpatch-native-fence-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let mut session = Session::new(&directory).unwrap();
    session.attach_layout_host().unwrap();
    let layout = session.project_document().layout.clone();
    session
        .publish_host_layout(
            session.revision(),
            session.state_id(),
            session.layout_generation(),
            layout,
            LayoutIntent::Placement,
        )
        .unwrap();
    (directory, session.into_shared())
}

fn publish_now(
    session: &SharedSession,
    edit: impl FnOnce(&mut ProjectLayout),
    intent: LayoutIntent,
) {
    let mut live = session.lock().unwrap();
    let mut layout = live.project_document().layout.clone();
    edit(&mut layout);
    let (revision, state_id, generation) =
        (live.revision(), live.state_id(), live.layout_generation());
    live.publish_host_layout(revision, state_id, generation, layout, intent)
        .unwrap();
}

fn add_constant(
    session: &SharedSession,
    fence: DocumentExecutionFence,
) -> Result<DocumentExecutionFence, String> {
    execute_shared_fenced(
        session,
        "apply_edits",
        json!({"edits": [{"tool": "add_module", "arguments": {"id": "dc", "kind": "constant"}}],
               "expected_revision": fence.revision}),
        fence,
    )
    .map(|(_, next)| next)
}

#[test]
fn native_build_fence_allows_camera_moves_after_preparation() {
    // The rack publishes its camera (pan/zoom) whenever navigation settles; a
    // build prepared just before must still apply.
    let (directory, session) = placed_session("camera");
    let fence = DocumentExecutionFence::capture(&session.lock().unwrap());
    publish_now(&session, |l| l.pan = [40.0, -25.0], LayoutIntent::Camera);
    publish_now(&session, |l| l.zoom = 0.6, LayoutIntent::Camera);
    let next = add_constant(&session, fence).unwrap();
    // the added module's pending placement completes, then the camera moves again
    let placed = {
        let live = session.lock().unwrap();
        let mut layout = live.project_document().layout.clone();
        layout.positions.insert("dc".into(), [0.0, 0.0]);
        layout
    };
    publish_now(&session, |l| *l = placed, LayoutIntent::Placement);
    publish_now(&session, |l| l.pan = [0.0, 0.0], LayoutIntent::Camera);
    execute_shared_fenced(
        &session,
        "start_construction",
        construction_request(next.revision),
        next,
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn native_build_fence_still_rejects_moved_modules() {
    let (directory, session) = placed_session("geometry");
    let fence = DocumentExecutionFence::capture(&session.lock().unwrap());
    add_constant(&session, fence).unwrap();
    let at = |x: f32| {
        move |l: &mut ProjectLayout| {
            l.positions.insert("dc".into(), [x, 0.0]);
        }
    };
    publish_now(&session, at(0.0), LayoutIntent::Placement);
    let fence = DocumentExecutionFence::capture(&session.lock().unwrap());
    publish_now(&session, at(300.0), LayoutIntent::Placement);
    let error = execute_shared_fenced(
        &session,
        "start_construction",
        construction_request(fence.revision),
        fence,
    )
    .unwrap_err();
    assert!(error.contains("document changed"), "{error}");
    let _ = std::fs::remove_dir_all(directory);
}
