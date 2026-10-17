use super::*;

#[test]
fn rackmcp_host_status_and_unavailable_transport_do_not_publish_layout_history_or_graph() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[{"id":"seq","kind":"rackmcp_sequencer"}]}})
    ));
    let revision = app.revision;
    let state_id = app.state_id;
    let generation = app.shared.lock().unwrap().layout_generation();
    let history = app.shared.lock().unwrap().snapshot()["history"].clone();
    let document = app.capture_document();
    assert!(app.command("inspect_rackmcp_sequence", json!({"id":"seq"})));
    assert!(!app.command(
        "rackmcp_transport",
        json!({"id":"seq","action":"select","pattern":"A","quantize":"wrap"})
    ));
    assert!(!app.command(
        "rackmcp_transport",
        json!({"id":"seq","action":"seek","bar":2.5})
    ));
    let session = app.shared.lock().unwrap();
    assert_eq!(session.revision(), revision);
    assert_eq!(session.state_id(), state_id);
    assert_eq!(session.layout_generation(), generation);
    assert_eq!(session.snapshot()["history"], history);
    drop(session);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.revision, revision);
}
