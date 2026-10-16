fn stock_octave_callback_patch() -> Patch {
    serde_json::from_value(serde_json::json!({
        "version":2,
        "nodes":[
            {"id":"pitch","kind":"poly_constant","params":{"channels":3,"v1":0.0,"v2":0.25,"v3":0.5}},
            {"id":"cv","kind":"constant","params":{"value":1.6}},
            {"id":"oct","kind":"stock_octave","params":{"octaves":1.0}}
        ],
        "cables":[
            {"from":"pitch","to":"oct","input":"pitch"},
            {"from":"cv","output":"out","to":"oct","input":"shift_cv"}
        ],
        "output":"oct","output_port":"out"
    })).unwrap()
}

#[test]
fn stock_octave_callback_publishes_actual_channel_zero_and_has_no_heap_activity() {
    let patch = stock_octave_callback_patch();
    let (mut callback, _, _) = sequence_callback(&patch);
    assert!(callback.performance.module_lights("oct").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
    assert_eq!(callback.performance.module_lights("oct").unwrap().octave_shift, Some(0));
    assert_eq!(callback.performance.snapshot()["modules"]["oct"]["frame"], 0);
    let mut first_sample = [0.0_f32; 1];
    without_heap_activity(|| callback.render(&mut first_sample, 1));
    assert_eq!(callback.performance.module_lights("oct").unwrap().octave_shift, Some(1));
    without_heap_activity(|| callback.render(&mut first_sample, 1));
    assert_eq!(callback.performance.module_lights("oct").unwrap().octave_shift, Some(3));
    for frames in [1, 3, 64, 129] {
        let mut samples = vec![0.0_f32; frames];
        without_heap_activity(|| callback.render(&mut samples, 1));
        assert!(samples.iter().all(|sample| sample.is_finite()));
        let state = callback.performance.module_lights("oct").unwrap();
        assert_eq!(state.count, 0);
        assert_eq!(state.octave_shift, Some(3));
        assert_eq!(state.brightness, [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]);
    }
}

#[test]
fn stock_octave_callback_freezes_actual_display_on_bypass_without_allocating() {
    let mut patch = stock_octave_callback_patch();
    let (mut callback, mut updates, mut retired) = sequence_callback(&patch);
    let mut samples = [0.0_f32; 32];
    without_heap_activity(|| callback.render(&mut samples, 1));
    assert_eq!(callback.performance.module_lights("oct").unwrap().octave_shift, Some(3));

    patch.nodes[2].bypassed = true;
    callback.performance.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut samples, 1));
    while retired.pop().is_ok() {}
    assert_eq!(callback.performance.module_lights("oct").unwrap().octave_shift, Some(3));
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["oct"]["lights"]["octave_shift"], 3);
    assert_eq!(published["modules"]["oct"]["lights"]["count"], 0);
}
