use super::*;

#[test]
fn weak_scope_capture_uses_identical_raw_engine_data_in_auto_and_manual_views() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = host(directory.path());
    let automatic = app.prepare_weak_scope_fixture(true);
    let manual = app.prepare_weak_scope_fixture(false);
    assert_eq!(automatic.x, manual.x);
    assert_eq!(automatic.y, manual.y);
    assert_eq!(automatic.measurements(), manual.measurements());
    assert_eq!((automatic.x_channels, automatic.y_channels), (1, 1));
    assert_eq!(
        app.patch
            .nodes
            .iter()
            .find(|node| node.id == "scope")
            .unwrap()
            .params["auto_range"],
        0.0
    );
    assert!(app.audio.is_none());
}
