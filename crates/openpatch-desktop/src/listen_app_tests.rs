use super::*;

#[test]
fn listen_host_observation_and_unavailable_capture_do_not_publish_layout_or_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[{"id":"tap","kind":"rackmcp_listen"}]}})
    ));
    let revision = app.revision;
    let state_id = app.state_id;
    let generation = app.shared.lock().unwrap().layout_generation();
    let before = app.shared.lock().unwrap().snapshot()["history"].clone();
    let document = app.capture_document();
    assert!(app.command("inspect_listen", json!({"id":"tap"})));
    assert!(!app.command(
        "start_listen_capture",
        json!({"id":"tap","seconds":1.0,"write_wav":false})
    ));
    let session = app.shared.lock().unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(session.state_id(), state_id);
    assert_eq!(session.layout_generation(), generation);
    assert_eq!(session.snapshot()["history"], before);
    drop(session);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.revision, revision);
}
