#[test]
fn rackmcp_live_selection_changes_dirty_state_and_saved_capture_cleans_current_name() {
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{
        "id":"automation","kind":"rackmcp_sequencer",
        "rackmcp_sequence":{"data":{"patterns":[
            {"name":"A","length":4},{"name":"B♭","length":4}
        ]}}
    }]}))
    .unwrap();
    let mut rack = Rack::default();
    rack.reconcile(&patch, &openpatch_core::catalog());
    let original = Document::new(patch.clone(), rack.layout()).unwrap();
    let mut state = DocumentState::new(original.project(), 7, &rack);
    let selection = |name| {
        SavedModuleState::RackMcpSelection(openpatch_core::FixedPatternName::new(name).unwrap())
    };
    let live = |name| Some(BTreeMap::from([("automation".into(), selection(name))]));
    assert!(!state.observe_saved_runtime(original.project(), 7, &rack, live("A")));
    assert!(!state.dirty());
    assert!(state.observe_saved_runtime(original.project(), 7, &rack, live("B♭")));
    assert!(
        state.dirty(),
        "live selection changes persisted content at the same document revision"
    );
    let mut captured = patch.clone();
    selection("B♭").write_node(&mut captured.nodes[0]);
    let saved = Document::new(captured, rack.layout()).unwrap();
    state.mark_saved(&saved);
    assert!(
        !state.dirty(),
        "saving the current selected name establishes a clean baseline"
    );
    assert!(state.matches_saved(&saved));
    assert!(!state.observe_saved_runtime(original.project(), 7, &rack, live("B♭")));
    assert!(state.observe_saved_runtime(original.project(), 7, &rack, live("A")));
    assert!(
        state.dirty(),
        "another live selection dirties the saved B pattern"
    );
    state.mark_saved(&saved);
    assert!(
        state.dirty(),
        "completion of an older B save cannot hide the current A selection"
    );
}

#[test]
fn attached_save_authority_tracks_completion_without_copying_the_private_startup_baseline() {
    let patch: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"dc","kind":"constant"}]}))
            .unwrap();
    let mut rack = Rack::default();
    rack.reconcile(&patch, &openpatch_core::catalog());
    let document = Document::new(patch, rack.layout()).unwrap();
    let mut state = DocumentState::new(document.project(), 1, &rack);
    assert!(!state.dirty());
    let shared = SharedProjectSave::default();
    state.attach_baseline(shared.clone());
    assert!(
        state.dirty(),
        "attaching must not implicitly bless the active shared document"
    );
    let initial = document_content(&document);
    shared.mark_initial(initial.clone()).unwrap();
    assert!(
        !state.dirty(),
        "desktop reads the shared authority immediately"
    );
    let mut later = initial.clone();
    later["document"]["layout"]["pan"] = json!([80, 0]);
    state.observe_canonical(Some(later.clone()));
    assert!(state.dirty());
    shared
        .register(
            &ProjectSaveCheckpoint {
                generation: shared.generation(),
                revision: 1,
                frame: None,
                edit_content: initial.clone(),
            },
            Some("saved.json".into()),
        )
        .unwrap();
    assert!(
        state.dirty(),
        "completion of an earlier snapshot leaves later edits dirty"
    );
    shared
        .register(
            &ProjectSaveCheckpoint {
                generation: shared.generation(),
                revision: 2,
                frame: None,
                edit_content: later.clone(),
            },
            Some("saved.json".into()),
        )
        .unwrap();
    assert!(!state.dirty());
    assert!(state.observe_canonical(None));
    assert!(
        state.dirty(),
        "unavailable durable observation must never claim clean"
    );
    assert!(state.observe_canonical(Some(later)));
    assert!(!state.dirty());
}
