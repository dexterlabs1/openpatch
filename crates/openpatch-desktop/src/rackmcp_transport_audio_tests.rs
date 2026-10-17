fn rackmcp_transport_patch() -> Patch {
    let mut patch = sequence_load_patch();
    patch.nodes[0]
        .rackmcp_sequence
        .as_mut()
        .unwrap()
        .data
        .patterns
        .push(serde_json::from_value(serde_json::json!({"name":"B♭","length":4})).unwrap());
    patch.nodes[0].params.insert("running".into(), 0.0);
    patch
}

fn rackmcp_transport_data(patch: &Patch) -> &openpatch_core::rackmcp_sequence::SequenceData {
    &patch.nodes[0].rackmcp_sequence.as_ref().unwrap().data
}

#[test]
fn rackmcp_transport_callback_publishes_actual_status_and_selected_saved_name_without_heap() {
    use openpatch_core::RackMcpTransport as T;
    let patch = rackmcp_transport_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    let unpublished = backend.snapshot();
    assert_eq!(unpublished["modules"]["automation"]["available"], false);
    assert!(unpublished["modules"]["automation"]["rackmcp_sequence"].is_null());
    let receipt = backend
        .rackmcp_transport(
            "automation",
            &[
                T::Select {
                    pattern: 1,
                    now: true,
                },
                T::Play,
            ],
            rackmcp_transport_data(&patch),
        )
        .unwrap();
    assert_eq!(receipt["queued"], true);
    assert_eq!(receipt["applied"], false);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
    let snapshot = backend.snapshot();
    assert_eq!(snapshot["coherent"], true);
    let module = &snapshot["modules"]["automation"];
    assert_eq!(module["state"]["selected_pattern"], "B♭");
    assert_eq!(module["rackmcp_sequence"]["running"], true);
    assert_eq!(module["rackmcp_sequence"]["selected_pattern_name"], "B♭");
    assert_eq!(module["rackmcp_sequence"]["active_pattern_name"], "A");
    assert_eq!(module["rackmcp_sequence"]["pending_pattern_name"], "B♭");
    assert_eq!(module["parameters"]["running"], 1.0);
    // Source `now` selects on the next advance, 6000 samples at 48 kHz,
    // 120 BPM and four steps per beat; the first publication remains on A.
    without_heap_activity(|| callback.render(&mut [0.0_f32; 4096], 1));
    let advanced = backend.snapshot();
    assert_eq!(
        advanced["modules"]["automation"]["rackmcp_sequence"]["active_pattern_name"],
        "B♭"
    );
    assert!(
        advanced["modules"]["automation"]["rackmcp_sequence"]["pending_pattern_name"].is_null()
    );
    backend
        .rackmcp_transport(
            "automation",
            &[T::Seek { bar: 3.0 }],
            rackmcp_transport_data(&patch),
        )
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert!(
        backend.snapshot()["modules"]["automation"]["rackmcp_sequence"]["bar"]
            .as_f64()
            .unwrap()
            >= 3.0
    );
    backend
        .rackmcp_transport("automation", &[T::Stop], rackmcp_transport_data(&patch))
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert_eq!(
        backend.snapshot()["modules"]["automation"]["rackmcp_sequence"]["running"],
        false
    );
}

#[test]
fn rackmcp_transport_selection_capture_overlay_preserves_lane_override_across_rebuild() {
    use openpatch_core::RackMcpTransport as T;
    let mut patch = rackmcp_transport_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    backend
        .rackmcp_transport(
            "automation",
            &[
                T::Select {
                    pattern: 1,
                    now: true,
                },
                T::Play,
            ],
            rackmcp_transport_data(&patch),
        )
        .unwrap();
    render_sequence_lane(&mut callback, 0.25);
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_state("automation")
        .unwrap();
    let selected = callback
        .graphs
        .active
        .engine
        .module_saved_state(handle)
        .unwrap();
    selected.write_node(&mut patch.nodes[0]);
    backend.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    assert_eq!(
        backend.snapshot()["modules"]["automation"]["state"]["selected_pattern"],
        "B♭"
    );
    while retired.pop().is_ok() {}
    sync_sequence_load(&callback, &patch, 3);
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.25);
    while retired.pop().is_ok() {}
    set_live_parameter(&mut callback, "vca", "level", 0.6);
    backend.sync_patch(&patch, 4);
    assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
    render_sequence_lane(&mut callback, 0.6);
}

#[test]
fn rackmcp_transport_fences_content_reorder_bypass_and_recreated_identity() {
    use openpatch_core::RackMcpTransport as T;
    for fence in ["content", "bypass", "recreate"] {
        let mut patch = rackmcp_transport_patch();
        let (mut callback, mut updates, _retired) = sequence_callback(&patch);
        let backend = callback.performance.clone();
        backend
            .rackmcp_transport(
                "automation",
                &[
                    T::Select {
                        pattern: 1,
                        now: true,
                    },
                    T::Play,
                ],
                rackmcp_transport_data(&patch),
            )
            .unwrap();
        match fence {
            "content" => patch.nodes[0]
                .rackmcp_sequence
                .as_mut()
                .unwrap()
                .data
                .patterns
                .swap(0, 1),
            "bypass" => patch.nodes[0].bypassed = true,
            _ => backend.retire_modules(&["automation".into()]),
        }
        backend.sync_patch(&patch, 2);
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2048], 1));
        assert_eq!(backend.snapshot()["applied"], 0);
        assert_eq!(backend.snapshot()["dropped"], 1);
    }
}

#[test]
fn rackmcp_transport_invalid_batch_and_suspended_document_never_enqueue() {
    use openpatch_core::RackMcpTransport as T;
    let patch = rackmcp_transport_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    let data = rackmcp_transport_data(&patch);
    for requests in [
        vec![],
        vec![T::Seek { bar: f64::NAN }],
        vec![T::Select {
            pattern: 2,
            now: true,
        }],
        vec![T::Play, T::Stop],
        vec![
            T::Select {
                pattern: 1,
                now: true,
            },
            T::Seek { bar: 1.0 },
        ],
        vec![
            T::Select {
                pattern: 1,
                now: true,
            },
            T::Seek { bar: -1.0 },
        ],
    ] {
        assert!(backend
            .rackmcp_transport("automation", &requests, data)
            .is_err());
    }
    assert!(backend.rackmcp_transport("vca", &[T::Play], data).is_err());
    assert_eq!(backend.snapshot()["queued"], 0);
    backend.request_document_execution(1, ExecutionDisposition::SuspendedUnresolved);
    assert!(backend
        .rackmcp_transport("automation", &[T::Play], data)
        .is_err());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    assert_eq!(backend.snapshot()["available"], false);
    assert_eq!(backend.snapshot()["queued"], 0);
}

#[test]
fn rackmcp_transport_queue_full_rejects_complete_batch_and_never_claims_applied() {
    use openpatch_core::RackMcpTransport as T;
    let patch = rackmcp_transport_patch();
    let (mut callback, _, _retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    let data = rackmcp_transport_data(&patch);
    for _ in 0..super::performance::EVENT_CAPACITY {
        assert_eq!(
            backend
                .rackmcp_transport("automation", &[T::Stop], data)
                .unwrap()["applied"],
            false
        );
    }
    assert!(backend
        .rackmcp_transport(
            "automation",
            &[
                T::Select {
                    pattern: 1,
                    now: true
                },
                T::Play
            ],
            data
        )
        .is_err());
    assert_eq!(backend.snapshot()["applied"], 0);
    for _ in 0..4 {
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1024], 1));
    }
    let snapshot = backend.snapshot();
    assert_eq!(
        snapshot["modules"]["automation"]["state"]["selected_pattern"],
        "A"
    );
    assert_eq!(
        snapshot["modules"]["automation"]["rackmcp_sequence"]["running"],
        false
    );
    assert_eq!(snapshot["backpressure"], 1);
}

#[test]
fn rackmcp_transport_pending_selection_survives_capture_suspend_rebuild_and_resume() {
    use openpatch_core::RackMcpTransport as T;
    let mut patch = rackmcp_transport_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let backend = callback.performance.clone();
    backend
        .rackmcp_transport("automation", &[T::Play], rackmcp_transport_data(&patch))
        .unwrap();
    render_sequence_lane(&mut callback, 0.25);
    backend
        .rackmcp_transport(
            "automation",
            &[T::Select {
                pattern: 1,
                now: false,
            }],
            rackmcp_transport_data(&patch),
        )
        .unwrap();
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    let before = backend.snapshot()["modules"]["automation"]["rackmcp_sequence"].clone();
    assert_eq!(before["selected_pattern_name"], "B♭");
    assert_eq!(before["pending_pattern_name"], "B♭");
    set_live_parameter(&mut callback, "vca", "level", 0.75);
    let handle = callback
        .graphs
        .active
        .engine
        .resolve_module_state("automation")
        .unwrap();
    callback
        .graphs
        .active
        .engine
        .module_saved_state(handle)
        .unwrap()
        .write_node(&mut patch.nodes[0]);
    backend.request_document_execution(2, ExecutionDisposition::SuspendedUnresolved);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    backend.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
    while retired.pop().is_ok() {}
    backend.request_document_execution(3, ExecutionDisposition::RunnableExact);
    backend.sync_patch(&patch, 3);
    assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
    render_sequence_lane(&mut callback, 0.75);
    let after = backend.snapshot()["modules"]["automation"]["rackmcp_sequence"].clone();
    assert_eq!(after["running"], true);
    assert_eq!(after["selected_pattern_name"], "B♭");
    assert_eq!(after["active_pattern_name"], "A");
    assert_eq!(after["pending_pattern_name"], "B♭");
}

#[test]
fn rackmcp_saved_selection_acknowledgment_preserves_fresh_load_first_advance_and_donor() {
    for fresh in [false, true] {
        let initial = sequence_load_selection_patch("A");
        let receiving = sequence_load_selection_patch("B");
        let (mut callback, mut updates, _retired) = sequence_callback(&initial);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 3000], 1));
        if fresh {
            callback.performance.retire_modules(&["automation".into()]);
        }
        sync_sequence_load(&callback, &receiving, 2);
        callback
            .performance
            .load_module_saved_states(&std::collections::BTreeMap::from([(
                "automation".into(),
                openpatch_core::SavedModuleState::from_node(&receiving.nodes[0]).unwrap(),
            )]));
        let mut graph = sequence_graph(&callback, &receiving, 2);
        if !fresh {
            let mut delta = HostLifecycleDelta::default();
            delta.modules.entry("automation".into()).or_default().reset = true;
            graph.host_lifecycle = PreparedHostLifecycle::prepare(&graph.engine, delta).unwrap();
        }
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        if !fresh {
            let donor = callback.graphs.previous.as_ref().unwrap();
            let handle = donor.engine.resolve_module_state("automation").unwrap();
            assert_eq!(
                donor.engine.module_saved_state(handle),
                Some(openpatch_core::SavedModuleState::RackMcpSelection(
                    openpatch_core::FixedPatternName::new("A").unwrap()
                )),
                "acknowledging the receiver's typed selection must not replay it on the donor"
            );
        }
        let mut pitch = [0.0_f32; 1200];
        without_heap_activity(|| callback.graphs.active.engine.process_block(&mut pitch));
        assert_eq!(pitch[0], 0.0);
        assert_eq!(pitch[1199], 3.0, "saved-state acknowledgment must retain the fresh source load's first-advance B request");
    }
}
