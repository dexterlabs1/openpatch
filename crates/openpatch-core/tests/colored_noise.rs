use openpatch_core::{catalog, Engine, ModuleParameterActions, Node, Patch, PortFrame};
const OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];
fn patch(rate: u32, seed: f32, level: f32, port: &str) -> Patch {
    serde_json::from_value(serde_json::json!({"version":2,"sample_rate":rate,"nodes":[{"id":"noise","kind":"colored_noise","params":{"seed":seed,"level":level}}],"output":"noise","output_port":port})).unwrap()
}
fn frames(engine: &mut Engine, count: usize) -> Vec<PortFrame> {
    let mut result = vec![PortFrame::default(); count];
    engine.process_poly_block(&mut result);
    result
}

#[test]
fn catalog_declares_every_mono_output_voltage_parameter_and_state_policy() {
    let c = catalog();
    let d = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["kind"] == "colored_noise")
        .unwrap();
    assert_eq!(d["outputs"], serde_json::json!(OUTPUTS));
    assert_eq!(d["inputs"], serde_json::json!([]));
    assert_eq!(d["min_patch_version"], 2);
    assert_eq!(d["polyphony"]["maximum_channels"], 1);
    assert_eq!(d["params"]["seed"]["min"], 0.0);
    assert_eq!(d["params"]["seed"]["max"], 16777215.0);
    assert_eq!(d["params"]["seed"]["randomization"], "disabled");
    assert_eq!(d["params"]["level"]["randomization"], "linear");
    assert_eq!(d["bypass"]["routes"], serde_json::json!([]));
    for (i, output) in d["output_metadata"].as_array().unwrap().iter().enumerate() {
        assert_eq!(output["name"], OUTPUTS[i]);
        assert_eq!(output["units"], "V");
        assert_eq!(output["maximum_channels"], 1);
        assert!(output["nominal_rms_volts"].as_f64().unwrap() > 2.8);
    }
}

#[test]
fn parameters_inputs_ports_and_version_are_strictly_validated() {
    for name in ["seed", "level", "unknown"] {
        for value in [-1.0, f32::INFINITY, f32::NAN, 16777216.0] {
            let mut p = patch(48000, 1.0, 1.0, "white");
            p.nodes[0].params.insert(name.into(), value);
            assert!(p.compile().is_err());
        }
    }
    let mut p = patch(48000, 1.25, 1.0, "white");
    assert!(p.compile().unwrap_err().contains("integer"));
    p.nodes[0].params.insert("seed".into(), 16777215.0);
    assert!(p.compile().is_ok());
    p.nodes[0].params.insert("level".into(), 2.0001);
    assert!(p.compile().is_err());
    p.nodes[0].params.insert("level".into(), 0.0);
    assert!(p.compile().is_ok());
    p.version = 1;
    assert!(p.compile().is_err());
    p.version = 2;
    for output in ["out", "brown", "grey", "WHITE"] {
        p.output_port = output.into();
        assert!(p.compile().is_err());
    }
    p.output_port = "white".into();
    p.cables = serde_json::from_value(
        serde_json::json!([{"from":"noise","output":"white","to":"noise","input":"signal"}]),
    )
    .unwrap();
    assert!(p.compile().is_err());
}

#[test]
fn seed_rate_level_and_identity_adoption_work_for_every_output() {
    for output in OUTPUTS {
        let p = patch(48000, 17.0, 1.0, output);
        let mut old = p.compile().unwrap();
        frames(&mut old, 10001);
        let mut changed = p.clone();
        changed.nodes[0].params.insert("level".into(), 0.5);
        let mut continued = changed.compile().unwrap();
        continued.adopt_state_from(&old);
        let a = frames(&mut old, 4097);
        let b = frames(&mut continued, 4097);
        for (a, b) in a.iter().zip(b) {
            assert_eq!(b.channels, 1);
            assert_eq!(b.values[0], a.values[0] * 0.5);
            assert!(b.values[1..].iter().all(|v| *v == 0.0));
        }
        changed.nodes[0].params.insert("seed".into(), 18.0);
        let mut seeded = changed.compile().unwrap();
        seeded.adopt_state_from(&old);
        assert_eq!(
            frames(&mut seeded, 1025),
            frames(&mut changed.compile().unwrap(), 1025)
        );
        changed.sample_rate = 44100;
        changed.nodes[0].params.insert("seed".into(), 17.0);
        let mut rate = changed.compile().unwrap();
        rate.adopt_state_from(&old);
        assert_eq!(
            frames(&mut rate, 1025),
            frames(&mut changed.compile().unwrap(), 1025)
        );
        let mut fresh = p.clone();
        fresh.nodes[0].id = "other".into();
        fresh.output = Some("other".into());
        let mut renamed = fresh.compile().unwrap();
        renamed.adopt_state_from(&old);
        assert_eq!(
            frames(&mut renamed, 1025),
            frames(&mut fresh.compile().unwrap(), 1025)
        );
    }
}

#[test]
fn bypass_freezes_streams_and_initialize_and_incarnation_restart_all_histories() {
    for output in OUTPUTS {
        let p = patch(48000, 1.0, 1.0, output);
        let mut old = p.compile().unwrap();
        frames(&mut old, 257);
        let mut bypass = p.clone();
        bypass.nodes[0].bypassed = true;
        let mut paused = bypass.compile().unwrap();
        paused.adopt_state_from(&old);
        assert_eq!(frames(&mut paused, 1031), vec![PortFrame::mono(0.0); 1031]);
        let mut resumed = p.compile().unwrap();
        resumed.adopt_state_from(&paused);
        assert_eq!(frames(&mut resumed, 513), frames(&mut old, 513));
        let handle = resumed.resolve_module_state("noise").unwrap();
        assert!(resumed.initialize_module_state(handle));
        assert_eq!(
            frames(&mut resumed, 1025),
            frames(&mut p.compile().unwrap(), 1025)
        );
        assert!(resumed.reset_module_state(handle));
        assert_eq!(
            frames(&mut resumed, 1025),
            frames(&mut p.compile().unwrap(), 1025)
        );
    }
}

#[test]
fn output_selection_unused_ports_and_wiring_do_not_change_noise_stepping() {
    let p = patch(48000, 2917.0, 1.0, "pink");
    let mut old = p.compile().unwrap();
    frames(&mut old, 9001);
    let mut selected = p.clone();
    selected.output_port = "gray".into();
    let mut selected_engine = selected.compile().unwrap();
    selected_engine.adopt_state_from(&old);
    let mut reference = selected.compile().unwrap();
    frames(&mut reference, 9001);
    assert_eq!(
        frames(&mut selected_engine, 1025),
        frames(&mut reference, 1025)
    );
    let mut wired = selected.clone();
    wired.nodes.push(Node {
        content: None,
        score: None,
        bypassed: false,
        id: "unused".into(),
        kind: "poly_merge".into(),
        params: Default::default(),
    });
    wired.cables=serde_json::from_value(serde_json::json!([{"from":"noise","output":"black","to":"unused","input":"in1"},{"from":"noise","output":"blue","to":"unused","input":"in2"}])).unwrap();
    assert_eq!(
        frames(&mut wired.compile().unwrap(), 1025),
        frames(&mut selected.compile().unwrap(), 1025)
    );
}

#[test]
fn persistence_only_stores_parameters_and_initialize_randomize_use_catalog_policy() {
    let p = patch(48000, 33.0, 0.25, "red");
    let saved = serde_json::to_string(&p).unwrap();
    let restored: Patch = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        frames(&mut restored.compile().unwrap(), 257),
        frames(&mut p.compile().unwrap(), 257)
    );
    let mut node = p.nodes[0].clone();
    let actions = ModuleParameterActions::new(48000);
    actions.randomize(&mut node, 9128).unwrap();
    assert_eq!(node.params["seed"], 33.0);
    assert!((0.0..=2.0).contains(&node.params["level"]));
    actions.initialize(&mut node).unwrap();
    assert_eq!(node.params["seed"], 1.0);
    assert_eq!(node.params["level"], 1.0);
}
