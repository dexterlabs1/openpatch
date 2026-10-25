//! VCA-2 and Unity through the public engine: polyphony, CV, bypass, merge,
//! lights, initialize and the catalog contract.
use openpatch_core::{catalog, ModuleParameterActions, Patch, PortFrame};
use serde_json::{json, Value};

fn render(patch: &Value, frames: usize) -> Vec<PortFrame> {
    let patch: Patch = serde_json::from_value(patch.clone()).unwrap();
    let mut buffer = vec![PortFrame::default(); frames];
    patch.compile().unwrap().process_poly_block(&mut buffer);
    buffer
}

fn vca_patch(params: Value, port: &str, bypassed: bool) -> Value {
    json!({"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"voices","kind":"poly_constant","params":{"channels":4,"v1":8,"v2":-8,"v3":4,"v4":2}},
            {"id":"cv","kind":"poly_constant","params":{"channels":2,"v1":5,"v2":10}},
            {"id":"vca","kind":"stock_vca2","params":params,"bypassed":bypassed}],
        "cables":[
            {"from":"voices","output":"out","to":"vca","input":"in1"},
            {"from":"cv","output":"out","to":"vca","input":"lin1"},
            {"from":"cv","output":"out","to":"vca","input":"exp2"}],
        "output":"vca","output_port":port})
}

#[test]
fn vca2_channels_follow_their_signal_input() {
    let frames = render(&vca_patch(json!({"level1":0.5}), "out1", false), 4);
    let last = frames[3];
    // Four voices from IN; the two-voice LIN CV mutes voices three and four.
    assert_eq!(last.channels, 4);
    assert_eq!(&last.values[..4], &[2.0, -4.0, 0.0, 0.0]);
    // Channel 2 has only an exponential CV patched: one voice of 0 V.
    let frames = render(&vca_patch(json!({}), "out2", false), 4);
    assert_eq!(frames[3].channels, 1);
    assert_eq!(frames[3].values[0], 0.0);
}

#[test]
fn vca2_bypass_passes_each_input_to_its_output() {
    let frames = render(&vca_patch(json!({"level1":0.0}), "out1", true), 4);
    assert_eq!(frames[3].channels, 4);
    assert_eq!(&frames[3].values[..4], &[8.0, -8.0, 4.0, 2.0]);
}

fn unity_patch(params: Value, port: &str) -> Value {
    json!({"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"a","kind":"poly_constant","params":{"channels":3,"v1":6,"v2":1,"v3":1}},
            {"id":"b","kind":"poly_constant","params":{"channels":1,"v1":-2}},
            {"id":"unity","kind":"stock_unity","params":params}],
        "cables":[
            {"from":"a","output":"out","to":"unity","input":"ch1_1"},
            {"from":"b","output":"out","to":"unity","input":"ch1_6"},
            {"from":"b","output":"out","to":"unity","input":"ch2_2"}],
        "output":"unity","output_port":port})
}

#[test]
fn unity_sections_modes_inverse_and_merge() {
    let value = |params: Value, port: &str| {
        let frames = render(&unity_patch(params, port), 3);
        assert_eq!(frames[2].channels, 1);
        frames[2].values[0]
    };
    assert_eq!(value(json!({}), "mix1"), 4.0);
    assert_eq!(value(json!({}), "inv1"), -4.0);
    assert_eq!(value(json!({"average1":1}), "mix1"), 2.0);
    assert_eq!(value(json!({}), "mix2"), -2.0);
    assert_eq!(value(json!({"merge":1}), "mix2"), 2.0);
    assert_eq!(value(json!({"merge":1,"average2":1}), "inv2"), -(2.0 / 3.0));
}

#[test]
fn unity_lights_initialize_and_bypass() {
    let patch: Patch =
        serde_json::from_value(unity_patch(json!({"merge":1,"average1":1}), "mix1")).unwrap();
    let mut engine = patch.compile().unwrap();
    let mut buffer = vec![PortFrame::default(); 256];
    engine.process_poly_block(&mut buffer);
    let handle = engine.resolve_module_lights("unity").unwrap();
    let lights = engine.module_lights(handle).unwrap();
    assert_eq!(lights.count, 10);
    let row = |section: usize| -> Vec<f32> {
        (0..5)
            .map(|i| lights.brightness[section * 5 + i][0])
            .collect()
    };
    // Section 1 averages the merged 6, -2 and -2 V: 0.667 V, about -23.5 dB.
    // The -24 dB light is full, the -18 dB light just starting.
    let first = row(0);
    assert_eq!(&first[..3], &[0.0; 3]);
    assert!(first[3] > 0.0 && first[3] < 0.2, "{first:?}");
    assert_eq!(first[4], 1.0);
    // Section 2 sums the same merged inputs: 2 V, about -14 dB.
    let second = row(1);
    assert_eq!(&second[..2], &[0.0; 2]);
    assert!(second[2] > 0.5 && second[2] < 0.8, "{second:?}");
    assert_eq!(&second[3..], &[1.0, 1.0]);
    // Initialize returns the modes to sum and turns merge off; the meters keep
    // their level, as the source's reset handler leaves them.
    let mut reset = patch.clone();
    ModuleParameterActions::new(reset.sample_rate)
        .initialize(&mut reset.nodes[2])
        .unwrap();
    assert_eq!(
        reset.nodes[2].params.get("merge").copied().unwrap_or(0.0),
        0.0
    );
    assert_eq!(
        reset.nodes[2]
            .params
            .get("average1")
            .copied()
            .unwrap_or(0.0),
        0.0
    );
    let mut next = reset.compile().unwrap();
    next.adopt_state_from(&engine);
    let handle = next.resolve_module_lights("unity").unwrap();
    assert!(next.initialize_module_state(handle));
    assert_eq!(next.module_lights(handle).unwrap(), lights);
    // Bypass: Unity has no bypass route, so its outputs fall silent.
    let mut bypassed = patch;
    bypassed.nodes[2].bypassed = true;
    let mut buffer = vec![PortFrame::default(); 4];
    bypassed.compile().unwrap().process_poly_block(&mut buffer);
    assert_eq!(buffer[3].channel(0), 0.0);
}

#[test]
fn catalog_declares_ports_controls_and_source() {
    let catalog = catalog();
    let module = |kind: &str| {
        catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap()
            .clone()
    };
    let vca = module("stock_vca2");
    assert_eq!(
        vca["inputs"],
        json!(["exp1", "lin1", "in1", "exp2", "lin2", "in2"])
    );
    assert_eq!(vca["outputs"], json!(["out1", "out2"]));
    assert_eq!(vca["params"]["level1"]["default"], 1.0);
    assert!(vca["functional_reference"]
        .as_str()
        .unwrap()
        .ends_with("src/VCA.cpp"));
    assert_eq!(vca["bypass"]["routes"].as_array().unwrap().len(), 2);
    let unity = module("stock_unity");
    assert_eq!(unity["inputs"].as_array().unwrap().len(), 12);
    assert_eq!(unity["outputs"], json!(["mix1", "inv1", "mix2", "inv2"]));
    assert_eq!(unity["params"]["merge"]["source"], "module_data");
    assert_eq!(unity["light_names"].as_array().unwrap().len(), 10);
    assert!(unity["functional_reference"]
        .as_str()
        .unwrap()
        .ends_with("src/Unity.cpp"));
}

#[test]
fn duet_example_plays_both_amplifier_channels_through_unity() {
    let text = include_str!("../../../examples/legacy-vca2-unity-duet.json");
    let patch: Value = serde_json::from_str(text).unwrap();
    let frames = render(&patch, 48_000);
    assert!(frames
        .iter()
        .all(|f| f.values[..2].iter().all(|v| v.is_finite())));
    let peak = frames.iter().map(|f| f.values[0].abs()).fold(0.0, f32::max);
    assert!(peak > 0.05 && peak <= 1.0, "peak {peak}");
    for port in ["out1", "out2"] {
        let mut probe = patch.clone();
        probe["output"] = json!("amplifier");
        probe["output_port"] = json!(port);
        let peak = render(&probe, 48_000)
            .iter()
            .map(|f| f.values[0].abs())
            .fold(0.0, f32::max);
        assert!(peak > 0.5, "{port} peak {peak}");
    }
}
