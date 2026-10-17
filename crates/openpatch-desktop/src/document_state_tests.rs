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
