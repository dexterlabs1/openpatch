use openpatch_core::{catalog, Patch, PortDirection, PortFrame};
use serde_json::{json, Value};

fn mixer_patch(params: Value, input: &str, output: &str) -> Patch {
    serde_json::from_value(json!({
        "version": 2,
        "sample_rate": 48000,
        "nodes": [
            {"id": "source", "kind": "poly_constant", "params": {"channels": 1, "v1": 5}},
            {"id": "mix", "kind": "studio_mixer", "params": params}
        ],
        "cables": [{"from": "source", "output": "out", "to": "mix", "input": input}],
        "output": "mix",
        "output_port": output
    }))
    .unwrap()
}

fn last_sample(patch: &Patch) -> f32 {
    let mut engine = patch.compile().unwrap();
    let mut frames = [PortFrame::default(); 8];
    engine.process_poly_block(&mut frames);
    frames[7].values[0]
}

#[test]
fn catalog_exposes_exact_mixer_ports_channels_and_custom_mixing_tags() {
    let all = catalog();
    let mixer = all["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "studio_mixer")
        .unwrap();
    let inputs = mixer["inputs"].as_array().unwrap();
    assert_eq!(inputs.len(), 20);
    assert_eq!(inputs[0], "in1");
    assert_eq!(inputs[15], "in16");
    assert_eq!(mixer["inputs"][16], "return_a");
    assert_eq!(mixer["inputs"][17], "return_b");
    assert_eq!(mixer["inputs"][18], "send_a_cv");
    assert_eq!(mixer["inputs"][19], "send_b_cv");
    assert_eq!(
        mixer["outputs"],
        json!([
            "left",
            "right",
            "send_a_left",
            "send_a_right",
            "send_b_left",
            "send_b_right",
            "cue_left",
            "cue_right"
        ])
    );
    assert_eq!(mixer["tags"], json!(["Custom", "Mixing"]));
    assert_eq!(mixer["params"]["channels"]["min"], 1);
    assert_eq!(mixer["params"]["channels"]["max"], 16);
    assert_eq!(mixer["light_names"].as_array().unwrap().len(), 16);
    assert!(mixer["output_metadata"]
        .as_array()
        .unwrap()
        .iter()
        .all(|o| o["maximum_channels"] == 1));
}

#[test]
fn all_twenty_input_cables_and_eight_outputs_compile_and_have_port_lights() {
    let mut patch = mixer_patch(json!({"channels": 16}), "in1", "left");
    patch.cables.clear();
    for name in (1..=16)
        .map(|i| format!("in{i}"))
        .chain(["return_a", "return_b", "send_a_cv", "send_b_cv"].map(str::to_owned))
    {
        let cable = serde_json::from_value(json!({
            "from": "source", "output": "out", "to": "mix", "input": name
        }))
        .unwrap();
        patch.cables.push(cable);
    }
    for output in [
        "left",
        "right",
        "send_a_left",
        "send_a_right",
        "send_b_left",
        "send_b_right",
        "cue_left",
        "cue_right",
    ] {
        patch.output_port = output.into();
        let mut engine = patch.compile().unwrap();
        engine.set_port_lights_enabled(true);
        let handles = ["in16", "return_a", "return_b", "send_a_cv", "send_b_cv"].map(|port| {
            engine
                .resolve_port_light("mix", PortDirection::Input, port)
                .unwrap()
        });
        engine.process_poly_block(&mut [PortFrame::default(); 64]);
        for handle in handles {
            let light = engine.port_light(handle).unwrap();
            assert!(light.connected);
            assert_eq!(light.channels, 1);
        }
    }
}

#[test]
fn shrinking_hides_strip_safely_and_expanding_restores_its_wiring_and_settings() {
    let mut wide = mixer_patch(
        json!({"channels": 16, "pan16": 1, "send_a16": 1}),
        "in16",
        "right",
    );
    assert!(last_sample(&wide) > 0.1);
    let mut old = wide.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 64]);

    let mut narrow = wide.clone();
    narrow.nodes[1].params.insert("channels".into(), 1.0);
    let mut hidden = narrow.compile().unwrap();
    hidden.adopt_state_from(&old);
    let mut muted = [PortFrame::default(); 8];
    hidden.process_poly_block(&mut muted);
    assert!(muted.iter().all(|sample| sample.values[0] == 0.0));
    assert_eq!(narrow.cables[0].input, "in16");
    assert_eq!(narrow.nodes[1].params["pan16"], 1.0);

    let mut restored = wide.compile().unwrap();
    restored.adopt_state_from(&hidden);
    let mut audible = [PortFrame::default(); 8];
    restored.process_poly_block(&mut audible);
    assert!(audible.iter().any(|sample| sample.values[0] > 0.1));
    wide.nodes[1].bypassed = true;
    let mut bypassed = wide.compile().unwrap();
    bypassed.adopt_state_from(&restored);
    let mut silent = [PortFrame::default(); 8];
    bypassed.process_poly_block(&mut silent);
    assert!(silent.iter().all(|sample| sample.values[0] == 0.0));
}

#[test]
fn returns_do_not_feed_sends_and_send_cv_scales_only_send_output() {
    let returned = mixer_patch(
        json!({"channels": 1, "send_a1": 1}),
        "return_a",
        "send_a_left",
    );
    assert_eq!(last_sample(&returned), 0.0);
    let mut main = returned.clone();
    main.output_port = "left".into();
    assert!(last_sample(&main) > 0.1);

    let mut send = mixer_patch(json!({"channels": 1, "send_a1": 1}), "in1", "send_a_left");
    let full = last_sample(&send);
    let cv = serde_json::from_value(json!({
        "from": "source", "output": "out", "to": "mix", "input": "send_a_cv"
    }))
    .unwrap();
    send.cables.push(cv);
    let half = last_sample(&send);
    assert!(full > 0.1 && (half / full - 0.5).abs() < 0.01);
}
