use openpatch_core::*;
fn patch(level: Option<f32>, rate: u32) -> Patch {
    let source = Node {
        id: "source".into(),
        kind: "poly_constant".into(),
        params: [("channels".into(), 1.0), ("v1".into(), 10.0)].into(),
        bypassed: false,
        content: None,
        rackmcp_sequence: None,
        score: None,
    };
    let sum = Node {
        id: "sum".into(),
        kind: "poly_sum".into(),
        params: level
            .map(|v| [("level".into(), v)].into())
            .unwrap_or_default(),
        bypassed: false,
        content: None,
        rackmcp_sequence: None,
        score: None,
    };
    Patch {
        sample_rate: rate,
        nodes: vec![source, sum],
        cables: vec![Cable {
            from: "source".into(),
            to: "sum".into(),
            input: "signal".into(),
            output: "out".into(),
            id: None,
            color: None,
            input_order: None,
            output_order: None,
        }],
        output: Some("sum".into()),
        output_port: "out".into(),
        ..Patch::new_v2()
    }
}
#[test]
fn default_validation_persistence_and_partition() {
    assert!(patch(Some(-0.1), 48000).compile().is_err());
    assert!(patch(Some(1.1), 48000).compile().is_err());
    let p = patch(Some(0.25), 48000);
    let serialized = serde_json::to_string(&p).unwrap();
    let restored: Patch = serde_json::from_str(&serialized).unwrap();
    assert_eq!(restored.nodes[1].params["level"], 0.25);
    for rate in [8000, 48000, 96000, 192000] {
        let p = patch(None, rate);
        let mut a = p.compile().unwrap();
        let mut b = p.compile().unwrap();
        let mut frames = [PortFrame::default(); 1024];
        a.process_poly_block(&mut frames);
        for chunk in frames.chunks(17) {
            let mut out = vec![PortFrame::default(); chunk.len()];
            b.process_poly_block(&mut out);
            assert_eq!(chunk, out);
        }
        assert_eq!(frames[1023].values[0], 10.0);
        assert_eq!(
            a.module_lights(a.resolve_module_lights("sum").unwrap()),
            b.module_lights(b.resolve_module_lights("sum").unwrap())
        );
    }
}
#[test]
fn bypass_freezes_meter_and_adoption_preserves_divider() {
    let p = patch(None, 48000);
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 511]);
    let mut edited = patch(Some(0.5), 48000).compile().unwrap();
    edited.adopt_state_from(&old);
    edited.process_poly_block(&mut [PortFrame::default(); 1]);
    let lights = edited
        .module_lights(edited.resolve_module_lights("sum").unwrap())
        .unwrap();
    assert_eq!(lights.input_channels, Some(1));
    assert!(lights.brightness[3][0] > 0.9);
    let mut bypass = p.clone();
    bypass.nodes[1].bypassed = true;
    let mut muted = bypass.compile().unwrap();
    muted.adopt_state_from(&edited);
    let mut frames = [PortFrame::default(); 1024];
    muted.process_poly_block(&mut frames);
    assert!(frames.iter().all(|f| f.channels == 1 && f.values[0] == 0.0));
    assert_eq!(
        muted
            .module_lights(muted.resolve_module_lights("sum").unwrap())
            .unwrap(),
        lights
    );
}

#[test]
fn public_rate_change_resets_telemetry_and_catalog_exposes_contract() {
    let mut old = patch(None, 48000).compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 512]);
    let mut changed = patch(None, 96000).compile().unwrap();
    changed.adopt_state_from(&old);
    let lights = changed
        .module_lights(changed.resolve_module_lights("sum").unwrap())
        .unwrap();
    assert_eq!(lights.input_channels, Some(0));
    assert_eq!(lights.brightness, [[0.0; 2]; MAX_MODULE_LIGHTS]);
    let catalog = catalog();
    let sum = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "poly_sum")
        .unwrap();
    assert_eq!(sum["params"]["level"]["default"], 1.0);
    assert_eq!(sum["light_names"].as_array().unwrap().len(), 6);
    assert_eq!(sum["light_policy"]["publication_frames"], 512);
    assert_eq!(sum["light_policy"]["reference_voltage"], 10.0);
}
