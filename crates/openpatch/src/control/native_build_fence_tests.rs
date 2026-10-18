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
