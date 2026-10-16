// Actual offline Engine evidence for the connected factory. No audio device.
impl OpenPatch {
    fn prepare_stock_process_fixture(&mut self) -> openpatch_core::ScopeSnapshot {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-process-voice.json"))
                .expect("bundled Stock Process voice");
        let mut engine = patch.compile().expect("Stock Process voice compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 96_032];
        engine.process_audio_block(&mut audio);
        assert!(audio.iter().flatten().all(|v| v.is_finite() && v.abs() < 1.0));
        assert!(audio.iter().any(|v| v[0].abs() > 0.00001));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0].iter().any(|v| v.max.abs() > 0.01));
        assert!(scope.y[0].iter().any(|v| v.max.abs() > 0.01));
        let process = engine.resolve_module_state("process").unwrap();
        let state = engine.stock_process_snapshot(process).unwrap();
        assert_eq!(state.gate_param, 0.0);
        assert_eq!(state.gate_light_brightness, 0.0);
        let lights = patch.nodes.iter().filter_map(|node| {
            let handle = engine.resolve_module_lights(&node.id).ok()?;
            Some((node.id.clone(), engine.module_lights(handle)?))
        }).collect();
        let controls = patch.nodes.iter().filter_map(|node| {
            let handle = engine.resolve_module_control(&node.id).ok()?;
            Some((node.id.clone(), engine.module_control_state(handle)?))
        }).collect();
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack.restore_layout(&Self::stock_process_layout()).unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights, controls);
        self.rack.set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope.clone()))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Stock Process voice".into();
        self.status = "Actual offline Engine · restrained LFO pitch through Process GLIDE into Stock VCO · Scope X: source, Y: glide · manual lamp off, no live audio".into();
        scope
    }
}
