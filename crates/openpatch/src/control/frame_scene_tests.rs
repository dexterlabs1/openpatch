use super::*;

fn session() -> Session {
    let mut session = Session::new(&std::env::temp_dir()).unwrap();
    session
        .execute(
            "replace_patch",
            json!({"patch":{"version":2,"nodes":[{"id":"scene","kind":"frame_scene"}]}}),
        )
        .unwrap();
    session
}

fn edit(session: &mut Session, change: Value) -> Value {
    session
        .execute(
            "edit_frame_scene",
            json!({"id":"scene","edit":change,"expected_revision":session.revision()}),
        )
        .unwrap()
}

#[test]
fn frame_scene_stable_edits_grouped_history_reordering_and_stale_revisions_are_atomic() {
    let mut session = session();
    edit(
        &mut session,
        json!({"operation":"add","timestamp":10000,"values":[0,10000,20000,30000]}),
    );
    edit(
        &mut session,
        json!({"operation":"add","timestamp":30000,"values":[30000,20000,10000,0]}),
    );
    let before = session.snapshot();
    for timestamp in [40000, 50000] {
        let rev = session.revision();
        session.execute("edit_frame_scene",json!({"id":"scene","edit":{"operation":"update","keyframe_id":0,"timestamp":timestamp},"expected_revision":rev,"history_group":"drag-key0"})).unwrap();
    }
    let after = session.snapshot();
    assert_eq!(
        after["history"]["undo_steps"],
        before["history"]["undo_steps"].as_u64().unwrap() + 1
    );
    assert_eq!(
        after["patch"]["nodes"][0]["content"]["keyframes"][1]["id"],
        0
    );
    assert_eq!(
        session.execute("undo", json!({})).unwrap()["patch"],
        before["patch"]
    );
    assert_eq!(
        session.execute("redo", json!({})).unwrap()["patch"],
        after["patch"]
    );
    let now = session.snapshot();
    for change in [
        json!({"operation":"update","keyframe_id":0,"timestamp":30000}),
        json!({"operation":"remove","keyframe_id":999}),
        json!({"operation":"update","keyframe_id":0}),
        json!({"operation":"add","timestamp":12,"values":[1,2,3,65536]}),
    ] {
        assert!(session
            .execute("edit_frame_scene", json!({"id":"scene","edit":change}))
            .is_err());
        assert_eq!(session.snapshot(), now);
    }
    assert!(session
        .execute(
            "edit_frame_scene",
            json!({"id":"scene","edit":{"operation":"clear"},"expected_revision":0})
        )
        .is_err());
    assert_eq!(session.snapshot(), now);
}

#[test]
fn frame_scene_atomic_batch_content_presets_and_initialize_preserve_authored_data() {
    let mut session = session();
    edit(
        &mut session,
        json!({"operation":"add","timestamp":0,"values":[65535,32768,0,10000]}),
    );
    edit(
        &mut session,
        json!({"operation":"add","timestamp":65535,"values":[0,32768,65535,20000]}),
    );
    let before = session.snapshot();
    assert!(session.execute("apply_edits",json!({"edits":[
        {"tool":"set_parameter","arguments":{"id":"scene","parameter":"frame","value":0.5}},
        {"tool":"edit_frame_scene","arguments":{"id":"scene","edit":{"operation":"remove","keyframe_id":999}}}
    ]})).is_err());
    assert_eq!(session.snapshot(), before);
    let preset = session
        .execute("capture_module_preset", json!({"id":"scene"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["content"], before["patch"]["nodes"][0]["content"]);
    session
        .execute("initialize_modules", json!({"ids":["scene"]}))
        .unwrap();
    assert_eq!(
        session.snapshot()["patch"]["nodes"][0]["content"],
        before["patch"]["nodes"][0]["content"]
    );
    edit(&mut session, json!({"operation":"clear"}));
    session
        .execute("apply_module_preset", json!({"id":"scene","preset":preset}))
        .unwrap();
    assert_eq!(
        session.snapshot()["patch"]["nodes"][0]["content"],
        before["patch"]["nodes"][0]["content"]
    );
}
