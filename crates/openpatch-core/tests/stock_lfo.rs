use openpatch_core::*;
use serde_json::json;

fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":8192,
        "nodes":[{"id":"lfo","kind":"stock_lfo"}],"output":"lfo","output_port":"sine"}))
    .unwrap()
}
fn step(engine: &mut Engine) -> PortFrame {
    let mut output = [PortFrame::default()];
    engine.process_poly_block(&mut output);
    output[0]
}
fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("lfo").unwrap())
        .unwrap()
}
fn probe(params: serde_json::Value) -> StockLfoProbe {
    StockLfoProbe::new(
        &serde_json::from_value(json!({"id":"lfo","kind":"stock_lfo","params":params})).unwrap(),
        8192,
    )
    .unwrap()
}

#[test]
fn new_default_is_unipolar_and_old_lfos_retain_their_original_origins() {
    let mut stock = patch().compile().unwrap();
    assert_eq!(lights(&stock).clock_frequency, Some(1.0));
    let first = step(&mut stock);
    assert_eq!(first.channels, 1);
    assert!(first.values[0] >= 0.0 && first.values[0] < 0.0001);
    assert_eq!(lights(&stock).clock_frequency, Some(2.0));
    let mut old_patch = patch();
    old_patch.nodes[0].kind = "multi_lfo".into();
    let mut old = old_patch.compile().unwrap();
    assert_eq!(step(&mut old), PortFrame::mono(0.0));
    old_patch.output_port = "triangle".into();
    assert_eq!(
        step(&mut old_patch.compile().unwrap()),
        PortFrame::mono(-5.0)
    );
    old_patch.nodes[0].kind = "lfo".into();
    old_patch.version = 1;
    old_patch.output_port = "out".into();
    assert_eq!(
        step(&mut old_patch.compile().unwrap()),
        PortFrame::mono(0.5)
    );
}

#[test]
fn consumer_masks_taps_and_public_clock_count_state_are_actual_processed_values() {
    let mut p = patch();
    p.output = None;
    p.output_port = "out".into();
    let mut engine = p.compile().unwrap();
    assert!(engine.resolve_module_control("lfo").is_err());
    assert!(engine.resolve_module_event("lfo").is_err());
    let taps = std::array::from_fn::<_, 4, _>(|lane| {
        Some(
            engine
                .resolve_tap("lfo", ["sine", "triangle", "saw", "square"][lane], 0)
                .unwrap(),
        )
    });
    let mut audio = [[0.0; 2]; 32];
    let mut samples = [[TapSample::default(); 4]; 32];
    engine.process_audio_block_with_taps(&mut audio, &taps, &mut samples);
    assert!(samples
        .iter()
        .flatten()
        .all(|sample| sample.active && sample.value.is_finite()));
    let observed = lights(&engine);
    assert_eq!(observed.count, 5);
    assert_eq!(observed.clock_frequency, Some(2.0));
    assert_eq!(observed.brightness[4][0], 1.0);
    assert!(observed.brightness.iter().all(|v| v[1] == 0.0));
    let before = engine.resolve_tap("lfo", "sine", 0).unwrap();
    assert!(engine.resolve_tap("lfo", "out", 0).is_err());
    // A read-only tap handle adds no permanent consumer; only its process call does.
    let _ = before;
    step(&mut engine);
    assert_eq!(lights(&engine).clock_frequency, Some(2.0));
}

#[test]
fn engine_adoption_new_controls_initialize_rate_exclusion_and_bypass_are_bounded() {
    let mut p = patch();
    p.nodes[0].params.insert("rate".into(), 0.0);
    let mut old = p.compile().unwrap();
    old.process_poly_block(&mut [PortFrame::default(); 35]);
    let observed = lights(&old);
    let mut next = p.clone();
    next.nodes[0].params.insert("invert".into(), 1.0);
    let mut replacement = next.compile().unwrap();
    replacement.adopt_state_from(&old);
    assert_eq!(lights(&replacement), observed);
    let mut baseline = p.compile().unwrap();
    baseline.adopt_state_from(&old);
    let inverted = step(&mut replacement).values[0];
    let original = step(&mut baseline).values[0];
    assert!(
        (inverted - (10.0 - original)).abs() < 0.00001,
        "receiving inversion wins after adoption"
    );
    let handle = replacement.resolve_module_state("lfo").unwrap();
    let before = lights(&replacement);
    assert!(replacement.initialize_module_state(handle));
    assert_eq!(lights(&replacement).brightness, before.brightness);
    assert_eq!(lights(&replacement).clock_frequency, Some(1.0));
    assert!(step(&mut replacement).values[0] > 9.99);
    let mut bypass_patch = next.clone();
    bypass_patch.nodes[0].bypassed = true;
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&old);
    for _ in 0..40 {
        assert_eq!(step(&mut bypass), PortFrame::mono(0.0));
    }
    assert_eq!(lights(&bypass), observed);
    let mut fresh = next.compile().unwrap();
    let h = fresh.resolve_module_state("lfo").unwrap();
    fresh.adopt_state_from_excluding(&old, &[h]);
    assert_eq!(lights(&fresh).clock_frequency, Some(1.0));
    assert_eq!(lights(&fresh).brightness, [[0.0; 2]; 16]);
    next.sample_rate = 96000;
    let mut changed_rate = next.compile().unwrap();
    changed_rate.adopt_state_from(&old);
    assert_eq!(lights(&changed_rate).clock_frequency, Some(1.0));
}

#[test]
fn clock_is_shared_and_quartet_retirement_survives_reference_probe_adoption() {
    let mut lfo = probe(json!({"rate":0,"fm_amount":1}));
    let mut input = [PortFrame::default(); 4];
    input[0] = PortFrame {
        channels: 16,
        ..PortFrame::default()
    };
    input[3] = PortFrame::mono(0.0);
    for _ in 0..32 {
        lfo.step(&input, 15);
    }
    input[3].values[0] = 2.0;
    lfo.step(&input, 15);
    let observed = lfo.snapshot();
    assert!(observed.clock_frequency > 100.0);
    let mut next = probe(json!({"rate":1,"fm_amount":1}));
    next.adopt(&lfo);
    assert_eq!(next.snapshot(), observed);
    input[0].channels = 1;
    input[3].channels = 2;
    input[3].values[1] = 0.0;
    let before = next.snapshot();
    for _ in 0..8 {
        next.step(&input, 0);
    }
    let shrunk = next.snapshot();
    assert_eq!(&shrunk.phases[4..], &before.phases[4..]);
    assert_ne!(shrunk.phases[1], before.phases[1]);
    input[0].channels = 16;
    let output = next.step(&input, 15);
    assert!(output.iter().all(|f| f.channels == 16));
    assert_eq!(
        next.snapshot().clock_frequency,
        observed.clock_frequency,
        "lane one clock voltage never changes shared channel-zero rate"
    );
    let saved = next.snapshot();
    next.initialize();
    let initialized = next.snapshot();
    assert_eq!(initialized.phases, [0.0; 16]);
    assert_eq!(initialized.clock_timer, 0.0);
    assert_eq!(initialized.reset_high, saved.reset_high);
    assert_eq!(initialized.clock_trigger, saved.clock_trigger);
    assert_eq!(initialized.lights, saved.lights);
    assert_eq!(initialized.light_divider, saved.light_divider);
}

#[test]
fn catalog_parameters_actions_and_callbacks_preserve_explicit_contract() {
    let catalog = catalog();
    let m = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "stock_lfo")
        .unwrap();
    assert_eq!(m["inputs"], json!(["fm", "pwm", "reset", "clock"]));
    assert_eq!(m["outputs"], json!(["sine", "triangle", "saw", "square"]));
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"]["offset"]["default"], 1.0);
    assert_eq!(m["params"]["rate"]["default"], 1.0);
    assert_eq!(m["params"]["fm_amount"]["randomization"], "disabled");
    assert_eq!(m["params"]["pwm_amount"]["randomization"], "disabled");
    assert_eq!(
        m["light_names"],
        json!(["phase_red", "phase_green", "phase_blue", "invert", "offset"])
    );
    let actions = ModuleParameterActions::new(8192);
    let mut p = patch();
    p.nodes[0].params = [
        ("rate".into(), -8.0),
        ("fm_amount".into(), -0.7),
        ("pwm_amount".into(), 0.8),
    ]
    .into();
    actions.randomize(&mut p.nodes[0], 42).unwrap();
    assert_eq!(p.nodes[0].params["fm_amount"], -0.7);
    assert_eq!(p.nodes[0].params["pwm_amount"], 0.8);
    actions.initialize(&mut p.nodes[0]).unwrap();
    assert_eq!(p.nodes[0].params["offset"], 1.0);
    p.compile().unwrap();
    for (name, value) in [
        ("rate", -8.1),
        ("rate", 10.1),
        ("offset", 0.5),
        ("invert", f32::NAN),
        ("frequency", 2.0),
    ] {
        let mut invalid = patch();
        invalid.nodes[0].params.insert(name.into(), value);
        assert!(invalid.compile().is_err());
    }
    let mut invalid = patch();
    invalid.version = 1;
    assert!(invalid.compile().is_err());
    for rate in [8000, 44100, 48000, 96000, 192000] {
        let mut p = patch();
        p.sample_rate = rate;
        let mut whole = p.compile().unwrap();
        let mut parts = p.compile().unwrap();
        let mut expected = [PortFrame::default(); 1025];
        let mut actual = [PortFrame::default(); 1025];
        whole.process_poly_block(&mut expected);
        for chunk in actual.chunks_mut(17) {
            parts.process_poly_block(chunk);
        }
        assert_eq!(actual, expected);
        assert_eq!(lights(&whole), lights(&parts));
    }
}
