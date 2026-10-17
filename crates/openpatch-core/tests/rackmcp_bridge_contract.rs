//! Host-only Bridge shape and persistence, without any transport or device use.
use openpatch_core::{ModuleContent, Patch, PortFrame, ScoreData};
use serde_json::json;

fn bridge() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"host","kind":"rackmcp_bridge"}
    ]}))
    .unwrap()
}

#[test]
fn catalog_declares_host_observation_and_no_signal_controls_or_lamps() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "rackmcp_bridge")
        .unwrap();
    assert_eq!(module["host_only"], true);
    assert_eq!(module["model"], "host-only");
    assert_eq!(module["min_patch_version"], 2);
    for key in [
        "inputs",
        "outputs",
        "input_names",
        "output_names",
        "light_names",
    ] {
        assert_eq!(module[key], json!([]), "{key}");
    }
    assert_eq!(module["params"], json!({}));
    assert_eq!(module["host_status_policy"]["observation_ms"], 100);
    assert_eq!(module["host_status_policy"]["source_lamps_frames"], 512);
    assert_eq!(module["bypass"]["routes"], json!([]));
    assert!(module.get("content").is_none());
}

#[test]
fn graph_has_zero_ports_zero_dsp_lights_and_no_saved_runtime_state() {
    let patch = bridge();
    let mut engine = patch.compile().unwrap();
    let handle = engine.resolve_module_state("host").unwrap();
    let shape = engine.host_module_snapshot(handle).unwrap();
    assert_eq!((shape.input_count, shape.output_count), (0, 0));
    assert!(engine.resolve_tap("host", "out", 0).is_err());
    assert!(engine.resolve_module_lights("host").is_err());
    assert!(engine.resolve_module_control("host").is_err());
    assert!(engine.module_saved_state(handle).is_none());
    assert!(engine.listen_status(handle).is_none());
    assert!(engine.initialize_module_state(handle));
    assert!(!engine.reset_module_state(handle));
    let before = serde_json::to_value(&patch).unwrap();
    let mut output = [PortFrame::mono(99.0); 1024];
    engine.process_poly_block(&mut output);
    assert!(output
        .iter()
        .all(|frame| frame.values.iter().all(|v| *v == 0.0)));
    assert!(engine.module_saved_state(handle).is_none());
    assert_eq!(serde_json::to_value(&patch).unwrap(), before);
    for bypassed in [false, true] {
        let mut reopened: Patch = serde_json::from_value(before.clone()).unwrap();
        reopened.nodes[0].bypassed = bypassed;
        reopened.nodes.push({
            let mut duplicate = reopened.nodes[0].clone();
            duplicate.id = "copy".into();
            duplicate
        });
        let mut fresh = reopened.compile().unwrap();
        fresh.adopt_state_from(&engine);
        for id in ["host", "copy"] {
            let h = fresh.resolve_module_state(id).unwrap();
            assert_eq!(fresh.host_module_snapshot(h).unwrap().output_count, 0);
            assert!(fresh.module_saved_state(h).is_none());
        }
        fresh.process_poly_block(&mut output);
        assert!(output
            .iter()
            .all(|frame| frame.values.iter().all(|v| *v == 0.0)));
    }
}

#[test]
fn compiler_rejects_native_parameters_content_sequence_and_v1() {
    for key in [
        "resetPairing",
        "reset_pairing",
        "port",
        "clients",
        "writer",
        "running",
    ] {
        let mut patch = bridge();
        patch.nodes[0].params.insert(key.into(), 0.0);
        assert!(patch.compile().is_err(), "{key}");
    }
    let mut patch = bridge();
    patch.version = 1;
    assert!(patch.compile().is_err());
    let mut patch = bridge();
    patch.nodes[0].content = Some(ModuleContent::Notes {
        text: "host".into(),
    });
    assert!(patch.compile().is_err());
    let mut patch = bridge();
    patch.nodes[0].score = Some(ScoreData::default());
    assert!(patch.compile().is_err());
    let mut patch = bridge();
    patch.nodes[0].rackmcp_sequence = Some(Box::default());
    assert!(patch.compile().is_err());
    assert!(serde_json::from_value::<Patch>(json!({"version":2,"nodes":[
        {"id":"host","kind":"rackmcp_bridge","endpoint":"/tmp/forged.sock"}
    ]}))
    .is_err());
}

#[test]
fn bridge_cannot_be_selected_as_audio_or_connected_as_a_cable_endpoint() {
    let mut patch = bridge();
    patch.output = Some("host".into());
    assert!(patch.compile().is_err());
    for (from, to) in [("host", "signal"), ("signal", "host")] {
        let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"host","kind":"rackmcp_bridge"},
            {"id":"signal","kind":"gain"}
        ],"cables":[{"from":from,"to":to,"input":"signal"}]}))
        .unwrap();
        assert!(patch.compile().is_err());
    }
}
