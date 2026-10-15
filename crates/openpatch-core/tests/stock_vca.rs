use openpatch_core::*;
use serde_json::json;
fn patch(audio_width: u8, cv_width: u8, response: u8) -> Patch {
    let mut p:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,
      "nodes":[{"id":"audio","kind":"poly_constant","params":{"channels":audio_width,"v1":2,"v2":-4,"v3":6}},
               {"id":"cv","kind":"poly_constant","params":{"channels":cv_width,"v1":0,"v2":2.5,"v3":5,"v4":10,"v5":16}},
               {"id":"vca","kind":"stock_vca","params":{"response":response}}],
      "cables":[{"from":"audio","to":"vca","input":"signal"},{"from":"cv","to":"vca","input":"cv"}],
      "output":"vca"})).unwrap();
    p.assign_missing_cable_ids().unwrap();
    p
}
fn frame(e: &mut Engine) -> PortFrame {
    let mut f = [PortFrame::default()];
    e.process_poly_block(&mut f);
    f[0]
}
fn gains(e: &Engine) -> ModuleLightState {
    e.module_lights(e.resolve_module_lights("vca").unwrap())
        .unwrap()
}
#[test]
fn stock_vca_catalog_enum_and_parameter_actions_are_explicit() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["kind"] == "stock_vca")
        .unwrap();
    assert_eq!(m["inputs"], json!(["cv", "signal"]));
    assert_eq!(m["outputs"], json!(["out"]));
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"]["response"]["default"], 1.0);
    assert_eq!(m["params"]["response"]["randomization"], "enum");
    assert_eq!(m["input_metadata"][0]["normalization"], 10.0);
    assert_eq!(m["input_metadata"][1]["normalization"], 0.0);
    assert!(m["input_metadata"][0].get("normal_volts").is_none());
    for (name, value) in [
        ("response", 0.5),
        ("response", 2.0),
        ("level", -0.01),
        ("level", f32::NAN),
        ("other", 0.0),
    ] {
        let mut p = patch(1, 1, 1);
        p.nodes[2].params.insert(name.into(), value);
        assert!(p.compile().is_err());
    }
    let mut p = patch(1, 1, 1);
    p.version = 1;
    assert!(p.compile().is_err());
    let actions = ModuleParameterActions::new(48000);
    let mut p = patch(1, 1, 1);
    let mut seen = [false; 2];
    for seed in 0..64 {
        actions.randomize(&mut p.nodes[2], seed).unwrap();
        let r = p.nodes[2].params["response"];
        assert!(r == 0.0 || r == 1.0);
        seen[r as usize] = true;
        p.compile().unwrap();
    }
    assert_eq!(seen, [true, true]);
    actions.initialize(&mut p.nodes[2]).unwrap();
    assert_eq!(p.nodes[2].params["response"], 1.0);
    assert_eq!(p.nodes[2].params["level"], 1.0);
}
#[test]
fn stock_vca_delayed_cv_sets_width_and_mono_audio_broadcasts() {
    for (response, expected) in [
        (1, [0.0, 0.5, 1.0, 2.0, 2.0]),
        (0, [0.0, 0.0078125, 0.125, 2.0, 2.0]),
    ] {
        let mut e = patch(1, 5, response).compile().unwrap();
        assert_eq!(frame(&mut e), PortFrame::mono(0.0));
        let out = frame(&mut e);
        assert_eq!(out.channels, 5);
        assert_eq!(out.values[..5], expected);
        assert_eq!(out.values[5..], [0.0; 11]);
        let g = gains(&e);
        assert_eq!(g.input_channels, Some(5));
        for (i, v) in expected.iter().enumerate() {
            assert_eq!(g.brightness[i], [*v / 2.0, 0.0]);
        }
        assert_eq!(g.brightness[5..], [[0.0; 2]; 11]);
    }
}
#[test]
fn stock_vca_silent_unselected_module_still_reports_true_gain_and_zero_cv_differs_from_absent() {
    let mut p: Patch =
        serde_json::from_value(json!({"version":2,"nodes":[{"id":"vca","kind":"stock_vca"}]}))
            .unwrap();
    let mut e = p.compile().unwrap();
    assert_eq!(gains(&e).brightness, [[0.0; 2]; 16]);
    e.process_block(&mut [0.0; 17]);
    assert_eq!(gains(&e).input_channels, Some(1));
    assert_eq!(gains(&e).brightness[0], [1.0, 0.0]);
    p = patch(0, 5, 1);
    p.output = None;
    let mut e = p.compile().unwrap();
    e.process_block(&mut [0.0; 17]);
    assert_eq!(gains(&e).input_channels, Some(5));
    assert_eq!(gains(&e).brightness[3], [1.0, 0.0]);
    let taps = [Some(e.resolve_tap("vca", "out", 3).unwrap()), None];
    let mut samples = [[TapSample::default(); 2]; 3];
    e.process_audio_block_with_taps(&mut [[0.0; 2]; 3], &taps, &mut samples);
    assert!(samples.iter().all(|s| s[0].active && s[0].value == 0.0));
    let mut absent = patch(1, 0, 1).compile().unwrap();
    frame(&mut absent);
    assert_eq!(frame(&mut absent).values[0], 2.0);
    let mut zero = patch(1, 1, 1).compile().unwrap();
    frame(&mut zero);
    assert_eq!(frame(&mut zero).values[0], 0.0);
}
#[test]
fn stock_vca_bypass_uses_raw_audio_width_and_freezes_meter_across_cv_edits() {
    let p = patch(1, 5, 1);
    let mut active = p.compile().unwrap();
    active.process_block(&mut [0.0; 17]);
    let before = gains(&active);
    let mut p = p;
    p.nodes[2].bypassed = true;
    p.nodes[1].params.insert("channels".into(), 16.0);
    let mut bypass = p.compile().unwrap();
    bypass.adopt_state_from(&active);
    let mut values = [PortFrame::default(); 17];
    bypass.process_poly_block(&mut values);
    for v in values {
        assert_eq!(v, PortFrame::mono(2.0));
    }
    assert_eq!(gains(&bypass), before);
    p.nodes[2].bypassed = false;
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    resumed.process_block(&mut [0.0; 2]);
    assert_eq!(gains(&resumed).input_channels, Some(16));
}
#[test]
fn stock_vca_reset_initialize_rate_identity_and_adoption_keep_meter_lifecycle_clear() {
    let p = patch(1, 5, 1);
    let mut a = p.compile().unwrap();
    a.process_block(&mut [0.0; 17]);
    let before = gains(&a);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&a);
    assert_eq!(gains(&same), before);
    let h = same.resolve_module_state("vca").unwrap();
    assert!(same.initialize_module_state(h));
    assert_eq!(gains(&same).input_channels, Some(1));
    assert_eq!(gains(&same).brightness, [[0.0; 2]; 16]);
    frame(&mut same);
    assert_eq!(gains(&same), before);
    assert!(same.reset_module_state(h));
    assert_eq!(gains(&same).brightness, [[0.0; 2]; 16]);
    let mut other = p.clone();
    other.sample_rate = 96000;
    let mut other = other.compile().unwrap();
    other.adopt_state_from(&a);
    assert_eq!(gains(&other).brightness, [[0.0; 2]; 16]);
    let mut renamed = p;
    renamed.nodes[2].id = "new".into();
    for cable in &mut renamed.cables {
        cable.to = "new".into();
    }
    renamed.output = Some("new".into());
    let mut e = renamed.compile().unwrap();
    e.adopt_state_from(&a);
    assert_eq!(
        e.module_lights(e.resolve_module_lights("new").unwrap())
            .unwrap()
            .brightness,
        [[0.0; 2]; 16]
    );
}
