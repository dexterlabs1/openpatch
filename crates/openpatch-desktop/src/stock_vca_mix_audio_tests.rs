// Exercise the production callback and telemetry queue without an audio device.
fn stock_vca_mix_callback_patch() -> Patch {
    let mut patch: Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,"nodes":[
            {"id":"source","kind":"constant","params":{"value":1}},
            {"id":"mix","kind":"stock_vca_mix","params":{
                "level1":0.5,"mix_level":0.75,"channel_exp":1,"mix_exp":1}},
            {"id":"legacy","kind":"vca_mix","params":{"level1":0.75}}
        ],"cables":[{"from":"source","to":"mix","input":"in1"}],
        "output":"mix","output_port":"mix"
    })).unwrap();
    patch.assign_missing_cable_ids().unwrap();
    patch
}

fn stock_vca_mix_callback_state(callback: &Callback) -> openpatch_core::StockVcaMixSnapshot {
    let engine = &callback.graphs.active.engine;
    engine.stock_vca_mix_snapshot(engine.resolve_module_state("mix").unwrap()).unwrap()
}

#[test]
fn stock_vca_mix_callback_publishes_four_actual_lamps_and_freezes_them_in_bypass() {
    let mut patch = stock_vca_mix_callback_patch();
    // A mono signal is replicated into the padded quartet: channel1's actual
    // meter target is (1.25 * 4) / 5 = 1, hence full brightness at frame512.
    patch.nodes[0].params.insert("value".into(), 1.25);
    patch.nodes[1].params.insert("level1".into(), 1.0);
    let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
    assert!(callback.performance.module_lights("mix").is_none());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 511], 1));
    assert_eq!(callback.performance.module_lights("mix").unwrap().brightness[0], [0.0; 2]);
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
    let lights = callback.performance.module_lights("mix").unwrap();
    let before = stock_vca_mix_callback_state(&callback);
    assert_eq!(lights.count, 4);
    assert_eq!(lights.brightness[0], [1.0, 0.0]);
    assert_eq!(&lights.brightness[1..4], &[[0.0; 2]; 3]);
    let published = callback.performance.snapshot();
    assert_eq!(published["modules"]["mix"]["available"], true);
    assert_eq!(published["modules"]["mix"]["kind"], "stock_vca_mix");
    assert_eq!(published["modules"]["mix"]["lights"]["count"], 4);
    assert_eq!(published["modules"]["mix"]["frame"], 512);
    patch.nodes[1].bypassed = true;
    callback.performance.sync_patch(&patch, 2);
    assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
    without_heap_activity(|| callback.render(&mut [0.0_f32; 1024], 1));
    while disposal.pop().is_ok() {}
    assert_eq!(stock_vca_mix_callback_state(&callback), before);
    let frozen = callback.performance.module_lights("mix").unwrap();
    assert_eq!(frozen.count, lights.count);
    assert_eq!(frozen.brightness, lights.brightness);
}

#[test]
fn stock_vca_mix_callback_initialize_cannot_overwrite_later_edits_presets_or_undo() {
    for operation in ["batch", "later_edit", "preset", "undo"] {
        let patch = stock_vca_mix_callback_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let directory = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(directory.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session.set_live_module_events(Some(callback.performance.clone())).unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
        let before = stock_vca_mix_callback_state(&callback);
        if operation == "batch" {
            session.execute("apply_edits", serde_json::json!({"edits":[
                {"tool":"initialize_modules","arguments":{"ids":["mix"]}},
                {"tool":"set_parameter","arguments":{"id":"mix","parameter":"level1","value":0.5}},
                {"tool":"set_parameter","arguments":{"id":"mix","parameter":"mix_level","value":0}},
                {"tool":"set_parameter","arguments":{"id":"mix","parameter":"channel_exp","value":1}}
            ]})).unwrap();
        } else {
            session.execute("initialize_modules", serde_json::json!({"ids":["mix"]})).unwrap();
            match operation {
                "later_edit" => {
                    session.execute("set_parameter", serde_json::json!({"id":"mix","parameter":"mix_level","value":0})).unwrap();
                    session.execute("set_parameter", serde_json::json!({"id":"mix","parameter":"mix_exp","value":1})).unwrap();
                }
                "preset" => {
                    session.execute("apply_module_preset", serde_json::json!({"id":"mix","preset":{
                        "format":"openpatch-module-preset","version":1,"kind":"stock_vca_mix",
                        "params":{"mix_level":0,"level1":0.5,"channel_exp":1,"mix_exp":1}
                    }})).unwrap();
                }
                "undo" => { session.execute("undo", serde_json::json!({})).unwrap(); }
                _ => unreachable!(),
            }
        }
        let final_node = session.patch().nodes.iter().find(|n| n.id == "mix").unwrap();
        let expected = ["mix_level", "level1", "level2", "level3", "level4"]
            .map(|name| final_node.params.get(name).copied().unwrap_or(1.0));
        let expected_flags = ["channel_exp", "mix_exp"]
            .map(|name| final_node.params.get(name).copied().unwrap_or(0.0) == 1.0);
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert!(updates.push(graph).is_ok());
        // Zero-length callback accepts and initializes the receiving graph while
        // advancing neither meters nor divider: exact runtime preservation can
        // be checked independently of the newly authoritative gain controls.
        without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
        let receiving = stock_vca_mix_callback_state(&callback);
        assert_eq!(receiving.parameters, expected, "{operation}");
        assert_eq!([receiving.ch_exp, receiving.mix_exp], expected_flags, "{operation}");
        assert_eq!(receiving.meters, before.meters, "{operation}");
        assert_eq!(receiving.divider_clock, before.divider_clock, "{operation}");
        assert_eq!(receiving.light_brightness, before.light_brightness, "{operation}");
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1024], 1));
        while disposal.pop().is_ok() {}
        let values = read_raw_pair(&mut callback.graphs.active.engine, ("mix", "mix"), ("source", "out"));
        assert_eq!(values, [if operation == "undo" { 0.1875 } else { 0.0 }, 1.0], "{operation}");
        assert_eq!(session.patch().nodes.iter().find(|n| n.id == "legacy").unwrap().params["level1"], 0.75);
    }
}
