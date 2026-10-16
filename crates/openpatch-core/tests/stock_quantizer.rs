use openpatch_core::*;
use serde_json::json;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"q","kind":"stock_quantizer"}],"output":"q","output_port":"pitch"})).unwrap()
}
fn step(engine: &mut Engine) -> PortFrame {
    let mut output = [PortFrame::default()];
    engine.process_poly_block(&mut output);
    output[0]
}
fn display(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("q").unwrap())
        .unwrap()
}
#[test]
fn empty_input_actual_display_is_optional_metadata_without_fabricated_lamps() {
    let mut engine = patch().compile().unwrap();
    assert_eq!(display(&engine).playing_notes, None);
    assert_eq!(display(&engine).count, 0);
    assert!(engine.resolve_module_control("q").is_err());
    assert!(engine.resolve_module_event("q").is_err());
    assert_eq!(step(&mut engine), PortFrame::mono(0.0));
    assert_eq!(display(&engine).playing_notes, Some(1));
    assert_eq!(display(&engine).brightness, [[0.0; 2]; MAX_MODULE_LIGHTS]);
    assert!(engine.resolve_tap("q", "out", 0).is_err());
    let mut p = patch();
    p.output = None;
    p.output_port = "out".into();
    let mut disconnected = p.compile().unwrap();
    step(&mut disconnected);
    assert_eq!(
        display(&disconnected).playing_notes,
        Some(1),
        "unconsumed processor still observes pitches"
    );
    let tap = disconnected.resolve_tap("q", "pitch", 0).unwrap();
    let mut audio = [[0.0; 2]; 1];
    let mut samples = [[TapSample::default(); 1]; 1];
    disconnected.process_audio_block_with_taps(&mut audio, &[Some(tap)], &mut samples);
    assert!(samples[0][0].active);
}
#[test]
fn selected_output_and_delayed_cables_obey_all_poly_widths_and_empty_bank_fallback() {
    for width in 0..=16 {
        let mut p = patch();
        p.nodes[0].params.insert("scale_mask".into(), 0.0);
        p.nodes.push(serde_json::from_value(json!({"id":"pitch","kind":"poly_constant","params":{"channels":width,"v1":0,"v2":0.25,"v3":0.5,"v4":0.75}})).unwrap());
        p.cables =
            serde_json::from_value(json!([{"from":"pitch","to":"q","input":"pitch"}])).unwrap();
        let mut engine = p.compile().unwrap();
        step(&mut engine);
        let out = step(&mut engine);
        assert_eq!(out.channels, width.max(1) as u8);
        assert!(out.values.iter().all(|v| v.is_finite()));
        if width >= 4 {
            assert_eq!(&out.values[..4], &[0.0, 0.25, 0.5, 0.75]);
            assert_eq!(display(&engine).playing_notes, Some(585));
        }
        assert_eq!(
            engine.module_saved_state(engine.resolve_module_state("q").unwrap()),
            Some(SavedModuleState::StockQuantizerNotes(0))
        );
    }
}
#[test]
fn adoption_receives_bank_offset_and_explicit_saved_load_then_initialize_retains_observation() {
    let p = patch();
    let mut old = p.compile().unwrap();
    step(&mut old);
    let observed = display(&old).playing_notes;
    let mut next = p.clone();
    next.nodes[0].params.insert("scale_mask".into(), 8.0);
    next.nodes[0].params.insert("offset".into(), 0.25);
    let mut engine = next.compile().unwrap();
    engine.adopt_state_from(&old);
    let handle = engine.resolve_module_state("q").unwrap();
    assert_eq!(display(&engine).playing_notes, observed);
    assert_eq!(
        engine.module_saved_state(handle),
        Some(SavedModuleState::StockQuantizerNotes(8))
    );
    assert_eq!(step(&mut engine).values[0], 0.25);
    let playing = display(&engine).playing_notes;
    assert!(engine.set_module_saved_state(handle, SavedModuleState::StockQuantizerNotes(1)));
    assert_eq!(display(&engine).playing_notes, playing);
    assert_eq!(step(&mut engine).values[0], 0.0);
    let playing = display(&engine).playing_notes;
    assert!(engine.initialize_module_state(handle));
    assert_eq!(
        engine.module_saved_state(handle),
        Some(SavedModuleState::StockQuantizerNotes(4095))
    );
    assert_eq!(display(&engine).playing_notes, playing);
    assert_eq!(
        step(&mut engine).values[0],
        0.25,
        "runtime reset does not reset offset parameter"
    );
    next.sample_rate = 96000;
    let mut different_rate = next.compile().unwrap();
    different_rate.adopt_state_from(&old);
    assert_eq!(display(&different_rate).playing_notes, None);
}
#[test]
fn actual_pitch_bypass_copies_poly_values_and_freezes_display_then_resumes() {
    let mut p = patch();
    p.nodes[0].params.insert("offset".into(), 1.0);
    p.nodes[0].params.insert("scale_mask".into(), 1.0);
    p.nodes.push(serde_json::from_value(json!({"id":"pitch","kind":"poly_constant","params":{"channels":3,"v1":0.123,"v2":-0.44,"v3":1.7}})).unwrap());
    p.cables = serde_json::from_value(json!([{"from":"pitch","to":"q","input":"pitch"}])).unwrap();
    let mut old = p.compile().unwrap();
    for _ in 0..3 {
        step(&mut old);
    }
    let observed = display(&old);
    let mut bypasspatch = p.clone();
    bypasspatch.nodes[0].bypassed = true;
    let mut bypass = bypasspatch.compile().unwrap();
    bypass.adopt_state_from(&old);
    for _ in 0..5 {
        let out = step(&mut bypass);
        assert_eq!(out.channels, 3);
        assert_eq!(&out.values[..3], &[0.123, -0.44, 1.7]);
    }
    assert_eq!(display(&bypass), observed);
    let mut empty = patch();
    empty.nodes[0].bypassed = true;
    let mut empty = empty.compile().unwrap();
    assert_eq!(step(&mut empty), PortFrame::mono(0.0));
    assert_eq!(display(&empty).playing_notes, None);
}
#[test]
fn catalog_state_randomization_and_generic_quantizer_remain_separate() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_quantizer")
        .unwrap();
    assert_eq!(m["inputs"], json!(["pitch"]));
    assert_eq!(m["outputs"], json!(["pitch"]));
    assert_eq!(m["params"]["offset"]["min"], -1.0);
    assert_eq!(m["params"]["scale_mask"]["hidden"], true);
    assert_eq!(
        m["params"]["scale_mask"]["randomization"],
        "inclusive_integer"
    );
    assert_eq!(
        m["bypass"]["routes"],
        json!([{"input":"pitch","output":"pitch"}])
    );
    let mut p = patch();
    let actions = ModuleParameterActions::new(48000);
    actions.randomize(&mut p.nodes[0], 19).unwrap();
    assert!((0.0..=4095.0).contains(&p.nodes[0].params["scale_mask"]));
    assert_eq!(p.nodes[0].params["scale_mask"].fract(), 0.0);
    actions.initialize(&mut p.nodes[0]).unwrap();
    assert_eq!(p.nodes[0].params["offset"], 0.0);
    assert_eq!(p.nodes[0].params["scale_mask"], 4095.0);
    let state = SavedModuleState::StockQuantizerNotes(5);
    state.write_params(&mut p.nodes[0].params);
    assert_eq!(SavedModuleState::from_node(&p.nodes[0]), Some(state));
    assert_eq!(state.boolean(), None);
    assert_eq!(state.with_value(false), state);
    p.version = 1;
    assert!(p.compile().is_err());
    p.version = 2;
    for (name, value) in [
        ("scale_mask", 0.5),
        ("scale_mask", 4096.0),
        ("offset", 1.01),
        ("offset_cv", 0.0),
    ] {
        let mut bad = p.clone();
        bad.nodes[0].params.insert(name.into(), value);
        assert!(bad.compile().is_err());
    }
    let legacy:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"q","kind":"quantizer","params":{"scale_mask":0,"offset":0.1}}],"output":"q"})).unwrap();
    assert_eq!(
        step(&mut legacy.compile().unwrap()).values[0],
        0.1,
        "legacy empty bank still passes adjusted pitch through"
    );
}
