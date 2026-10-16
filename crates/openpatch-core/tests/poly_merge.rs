use openpatch_core::*;
use serde_json::json;

fn patch(selection: Option<f32>, source_channels: u8) -> Patch {
    let mut patch: Patch = serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"source","kind":"poly_constant","params":{"channels":source_channels,"v1":5,"v2":-13}},
                 {"id":"merge","kind":"poly_merge"}],
        "cables":[{"from":"source","to":"merge","input":"in16"}],
        "output":"merge"
    })).unwrap();
    if let Some(selection) = selection {
        patch.nodes[1].params.insert("channels".into(), selection);
    }
    patch
}

fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("merge").unwrap())
        .unwrap()
}

fn step(engine: &mut Engine) -> PortFrame {
    let mut frame = [PortFrame::default()];
    engine.process_poly_block(&mut frame);
    frame[0]
}

#[test]
fn legacy_configs_validation_and_saved_encoding() {
    for selection in [
        None,
        Some(0.0),
        Some(-1.0),
        Some(1.0),
        Some(7.0),
        Some(16.0),
    ] {
        let original = patch(selection, 16);
        let saved = serde_json::to_string(&original).unwrap();
        let restored: Patch = serde_json::from_str(&saved).unwrap();
        assert_eq!(original.nodes[1].params, restored.nodes[1].params);
        let mut engine = restored.compile().unwrap();
        assert_eq!(lights(&engine).output_channels, Some(0));
        assert_eq!(lights(&engine).automatic_channels, Some(0));
        let first = step(&mut engine);
        let expected = match selection {
            None | Some(0.0) => 16,
            Some(-1.0) => 0,
            Some(n) => n as u8,
        };
        assert_eq!(
            first.channels,
            if selection.is_none() || selection == Some(0.0) {
                0
            } else {
                expected
            }
        );
        let output = step(&mut engine);
        assert_eq!(output.channels, expected);
        assert_eq!(lights(&engine).output_channels, Some(expected));
        assert_eq!(lights(&engine).automatic_channels, Some(16));
        assert_eq!(output.values[15], if expected == 16 { 5.0 } else { 0.0 });
        assert!(output.values[..15].iter().all(|v| *v == 0.0));
    }
    for invalid in [
        -2.0,
        -0.5,
        0.5,
        16.5,
        17.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
    ] {
        assert!(patch(Some(invalid), 1).compile().is_err(), "{invalid}");
    }
    // Other channel parsers keep their original unsigned domain.
    let mut invalid = patch(None, 1);
    invalid.nodes[0].params.insert("channels".into(), -1.0);
    assert!(invalid.compile().is_err());
}

#[test]
fn empty_sources_and_unconsumed_outputs_publish_actual_counts() {
    for (source_channels, automatic) in [(0, 0), (1, 16), (16, 16)] {
        let mut p = patch(Some(-1.0), source_channels);
        p.output = None;
        p.nodes[0].params.insert("v1".into(), 0.0);
        let mut engine = p.compile().unwrap();
        step(&mut engine);
        step(&mut engine);
        assert_eq!(lights(&engine).automatic_channels, Some(automatic));
        assert_eq!(lights(&engine).output_channels, Some(0));
        assert_eq!(lights(&engine).count, 0);
        assert_eq!(lights(&engine).brightness, [[0.0; 2]; 16]);
        let serialized = serde_json::to_value(lights(&engine)).unwrap();
        assert_eq!(serialized["automatic_channels"], automatic);
        assert_eq!(serialized["output_channels"], 0);
        assert!(serialized.get("input_channels").is_none());
    }
}

#[test]
fn adoption_new_configuration_fresh_incarnation_rate_and_bypass() {
    let mut old = patch(Some(7.0), 16).compile().unwrap();
    step(&mut old);
    step(&mut old);
    let observed = lights(&old);
    let mut edited = patch(Some(-1.0), 16).compile().unwrap();
    edited.adopt_state_from(&old);
    assert_eq!(lights(&edited), observed);
    assert_eq!(step(&mut edited), PortFrame::default());
    assert_eq!(lights(&edited).automatic_channels, Some(16));
    let mut bypass_patch = patch(Some(-1.0), 0);
    bypass_patch.nodes[1].bypassed = true;
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&old);
    for _ in 0..20 {
        step(&mut bypass);
    }
    assert_eq!(lights(&bypass), observed);
    let mut fresh = patch(None, 16).compile().unwrap();
    let handle = fresh.resolve_module_lights("merge").unwrap();
    fresh.adopt_state_from_excluding(&old, &[handle]);
    assert_eq!(lights(&fresh).output_channels, Some(0));
    assert_eq!(lights(&fresh).automatic_channels, Some(0));
    let mut rate_patch = patch(None, 16);
    rate_patch.sample_rate = 96000;
    let mut rate = rate_patch.compile().unwrap();
    rate.adopt_state_from(&old);
    assert_eq!(lights(&rate).output_channels, Some(0));
    assert_eq!(lights(&rate).automatic_channels, Some(0));
    let before = lights(&edited);
    let handle = edited.resolve_module_lights("merge").unwrap();
    assert!(edited.initialize_module_state(handle));
    assert_eq!(lights(&edited), before);
    assert_eq!(step(&mut edited).channels, 16);
}

#[test]
fn changing_actual_counts_retains_one_sample_per_cable() {
    fn chain(channels: u8) -> Patch {
        let mut p = patch(None, channels);
        p.nodes
            .push(serde_json::from_value(json!({"id":"next","kind":"poly_merge"})).unwrap());
        p.cables.push(
            serde_json::from_value(json!({"from":"merge","to":"next","input":"in7"})).unwrap(),
        );
        p.output = Some("next".into());
        p
    }
    let mut old = chain(16).compile().unwrap();
    assert_eq!(step(&mut old).channels, 0);
    assert_eq!(step(&mut old).channels, 0);
    assert_eq!(step(&mut old).channels, 7);
    let mut shrunk = chain(0).compile().unwrap();
    shrunk.adopt_state_from(&old);
    assert_eq!(step(&mut shrunk).channels, 7);
    assert_eq!(step(&mut shrunk).channels, 7);
    assert_eq!(step(&mut shrunk), PortFrame::default());
    assert_eq!(lights(&shrunk).automatic_channels, Some(0));
    let mut regrown = chain(1).compile().unwrap();
    regrown.adopt_state_from(&shrunk);
    assert_eq!(step(&mut regrown).channels, 0);
    assert_eq!(step(&mut regrown).channels, 0);
    assert_eq!(step(&mut regrown).channels, 7);
    assert_eq!(lights(&regrown).automatic_channels, Some(16));
}

#[test]
fn callback_partition_and_catalog_parameter_actions() {
    for rate in [8000, 48000, 96000, 192000] {
        let mut p = patch(None, 16);
        p.sample_rate = rate;
        let mut whole = p.compile().unwrap();
        let mut pieces = p.compile().unwrap();
        let mut output = [PortFrame::default(); 1025];
        whole.process_poly_block(&mut output);
        for expected in output.chunks(17) {
            let mut actual = [PortFrame::default(); 17];
            pieces.process_poly_block(&mut actual[..expected.len()]);
            assert_eq!(expected, &actual[..expected.len()]);
        }
        assert_eq!(lights(&whole), lights(&pieces));
    }
    let catalog = catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "poly_merge")
        .unwrap();
    let spec = &module["params"]["channels"];
    assert_eq!(spec["min"], -1.0);
    assert_eq!(spec["max"], 16.0);
    assert_eq!(spec["step"], 1.0);
    assert_eq!(spec["default"], 0.0);
    assert_eq!(
        spec["values"],
        json!([0, -1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
    );
    assert_eq!(
        spec["labels"],
        json!([
            "Auto", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14",
            "15", "16"
        ])
    );
    assert_eq!(spec["randomization"], "disabled");
    let actions = ModuleParameterActions::new(48000);
    let mut node = patch(Some(-1.0), 16).nodes.remove(1);
    actions.randomize(&mut node, 31).unwrap();
    assert_eq!(node.params["channels"], -1.0);
    actions.initialize(&mut node).unwrap();
    assert_eq!(node.params["channels"], 0.0);
    assert!(module["light_names"].as_array().unwrap().is_empty());
}
