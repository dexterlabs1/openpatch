// Actual native Callback::render/telemetry pipeline; no device or window opens.
#[test]
fn random_native_callback_publishes_real_lights_across_revisions_without_heap() {
    for voices in [1, 16] {
        let mut patch: Patch = serde_json::from_value(serde_json::json!({
            "version":2,"sample_rate":48000,
            "nodes":[{"id":"values","kind":"poly_constant","params":{"channels":voices}},
                     {"id":"motion","kind":"random","params":{"rate":10,"shape":0.25,"spread":0.75,"unipolar":1}}],
            "cables":[{"from":"values","to":"motion","input":"external"}],
            "output":"motion","output_port":"linear"
        })).unwrap();
        patch.assign_missing_cable_ids().unwrap();
        let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
        assert!(callback.performance.module_lights("motion").is_none());
        for revision in 1..=3 {
            if revision > 1 {
                patch.nodes[1].params.insert("shape".into(), 0.75);
                patch.nodes[1].params.insert("unipolar".into(), 0.0);
                patch.nodes[1].bypassed = revision == 3;
                callback.performance.sync_patch(&patch, revision);
                assert!(callback.performance.module_lights("motion").is_none());
                assert!(updates
                    .push(sequence_graph(&callback, &patch, revision))
                    .is_ok());
            }
            // Finish the host's graph crossfade before queuing another revision.
            let mut samples = [0.0_f32; 512];
            without_heap_activity(|| callback.render(&mut samples, 2));
            assert!(samples.iter().all(|sample| sample.is_finite()));
            assert_eq!(callback.graphs.active.revision, revision);
            let engine = &callback.graphs.active.engine;
            let expected = engine
                .module_lights(engine.resolve_module_lights("motion").unwrap())
                .unwrap();
            let actual = callback.performance.module_lights("motion").unwrap();
            assert_eq!(
                actual, expected,
                "published packet must describe this exact active graph"
            );
            assert_eq!(actual.count, 5);
            assert!(!actual.manual);
            if revision == 1 {
                assert!(
                    actual.brightness[0][0] > 1.0,
                    "raw event light must survive transport without clamping"
                );
                assert_eq!(actual.brightness[1][0], 0.25);
                assert_eq!(actual.brightness[4][0], 1.0);
            } else if revision == 2 {
                assert_eq!(actual.brightness[1][0], 0.75);
                assert_eq!(actual.brightness[4][0], 0.0);
            }
            let snapshot = callback.performance.snapshot();
            assert_eq!(snapshot["modules"]["motion"]["kind"], "random");
            assert_eq!(snapshot["modules"]["motion"]["revision"], revision);
            assert!(callback
                .performance
                .trigger("motion", ModuleAction::BPress)
                .is_err());
            while retired.pop().is_ok() {}
        }
    }
}
