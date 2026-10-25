use openpatch_core::{catalog, CrossMatterProbe, Engine, ModuleLightState, Node, Patch, PortFrame};
use serde_json::{json, Value};

/// Stock VCO carrier (saw) and modulator (square) into Cross Matter.
fn patch(params: Value, rate: u32) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":rate,
        "nodes":[
            {"id":"car","kind":"stock_vco","params":{"frequency":-12.0}},
            {"id":"mod","kind":"stock_vco","params":{"frequency":7.0}},
            {"id":"cross","kind":"cross_matter","params":params}],
        "cables":[
            {"from":"car","output":"saw","to":"cross","input":"carrier"},
            {"from":"mod","output":"square","to":"cross","input":"modulator"}],
        "output":"cross","output_port":"out"}))
    .unwrap()
}

fn render(engine: &mut Engine, count: usize) -> Vec<f32> {
    let mut frames = vec![PortFrame::default(); count];
    engine.process_poly_block(&mut frames);
    frames.iter().map(|f| f.channel(0)).collect()
}

fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("cross").unwrap())
        .unwrap()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn native_96k_host_matches_frozen_cpp_frames_after_fixed_latency() {
    // Knob settings whose host mapping is exact: algorithm 0.5 -> 0.5,
    // timbre 0.25, level1 0.5 -> drive 0.25 / MIDI 66, level2 0.75 -> 0.5625.
    // Volts k/4096 map to codec word k, and output words map back exactly.
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/cross-matter-host-golden.json")).unwrap();
    const LATENCY: usize = 63;
    for (case, carrier) in [("host_golden_external", 0.0), ("host_golden_internal", 1.0)] {
        let node = Node {
            id: "cross".into(),
            kind: "cross_matter".into(),
            params: [
                ("algorithm", 0.5),
                ("timbre", 0.25),
                ("level1", 0.5),
                ("level2", 0.75),
                ("carrier", carrier),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            content: None,
            rackmcp_sequence: None,
            score: None,
            bypassed: false,
        };
        let mut probe = CrossMatterProbe::new(&node, 96000).unwrap();
        let data = &fixture["cases"][case];
        let inputs: Vec<i64> = data["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let expected: Vec<i64> = data["outputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        let frames = inputs.len() / 2;
        let mut actual = Vec::with_capacity(2 * frames);
        for n in 0..frames + LATENCY {
            let mut ports = [PortFrame::default(); 6];
            if n < frames {
                ports[0] = PortFrame::mono(inputs[2 * n] as f32 / 4096.0);
                ports[1] = PortFrame::mono(inputs[2 * n + 1] as f32 / 4096.0);
            } else {
                ports[0] = PortFrame::mono(0.0);
                ports[1] = PortFrame::mono(0.0);
            }
            let out = probe.step(&ports, 0b11);
            if n < LATENCY {
                assert_eq!(out[0].channel(0), 0.0, "{case} pre-roll frame {n}");
                continue;
            }
            for output in out {
                let word = output.channel(0) * 4096.0;
                assert_eq!(word, word.round(), "{case}: non-codec output");
                actual.push(word as i64);
            }
        }
        assert_eq!(actual.len(), expected.len());
        let mismatch = actual.iter().zip(&expected).position(|(a, b)| a != b);
        assert_eq!(
            mismatch, None,
            "{case}: first mismatch at word {mismatch:?}"
        );
        assert!(expected.iter().any(|v| *v != 0));
    }
}

#[test]
fn default_patch_is_audible_and_bounded() {
    let mut engine = patch(json!({}), 48000).compile().unwrap();
    let out = render(&mut engine, 24000);
    let peak = out[4800..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
    // Saturated ring output reaches codec full scale (8 V); band-limited
    // resampling to 48 kHz adds its Gibbs overshoot (under 10%).
    assert!(peak > 0.5 && peak <= 8.8, "default peak {peak}");
}

#[test]
fn every_algorithm_region_sounds_different_from_its_neighbours() {
    // Region centres after the source pot curve: crossfade, fold, analog ring,
    // digital ring, XOR, comparator, vocoder; timbre fixed. A single shared
    // multiply would make neighbours nearly identical.
    let knobs = [0.02, 0.15, 0.3, 0.42, 0.55, 0.66, 0.8];
    let renders: Vec<Vec<f32>> = knobs
        .iter()
        .map(|&knob| {
            let mut engine = patch(
                json!({"algorithm":knob,"timbre":0.6,"carrier":0.0,"level1":0.7,"level2":0.7}),
                48000,
            )
            .compile()
            .unwrap();
            render(&mut engine, 24000)[14400..].to_vec()
        })
        .collect();
    let levels: Vec<f32> = renders.iter().map(|r| rms(r)).collect();
    // The source vocoder is much quieter than the cross-modulation paths.
    assert!(
        levels.iter().all(|&l| l > 0.1),
        "silent algorithm: {levels:?}"
    );
    let ratios: Vec<f32> = renders
        .windows(2)
        .map(|pair| {
            let difference: Vec<f32> = pair[0].iter().zip(&pair[1]).map(|(a, b)| a - b).collect();
            rms(&difference) / rms(&pair[0]).max(rms(&pair[1]))
        })
        .collect();
    // Analog and digital ring are the closest pair (about 0.23 here).
    assert!(
        ratios.iter().all(|&r| r > 0.2),
        "neighbouring algorithms too similar: {ratios:?}"
    );
}

#[test]
fn chunk_boundaries_and_graph_adoption_are_seamless() {
    let p = patch(json!({"algorithm":0.62,"timbre":0.3}), 48000);
    let expected = render(&mut p.compile().unwrap(), 1031);
    let mut engine = p.compile().unwrap();
    let mut offset = 0;
    for count in [1, 2, 29, 0, 30, 31, 128, 3, 97, 129, 581] {
        let samples = render(&mut engine, count);
        for (a, b) in samples.iter().zip(&expected[offset..offset + count]) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        offset += count;
        let mut replacement = p.compile().unwrap();
        replacement.adopt_state_from(&engine);
        engine = replacement;
    }
    assert_eq!(offset, 1031);
}

#[test]
fn bypass_routes_carrier_and_modulator_unchanged() {
    let p = patch(json!({"algorithm":0.85,"carrier":2.0}), 48000);
    let mut engine = p.compile().unwrap();
    render(&mut engine, 4801);
    let before = lights(&engine);
    assert_eq!(before.count, 2);

    let mut bypass_patch = p.clone();
    bypass_patch.nodes[2].bypassed = true;
    bypass_patch.output_port = "aux".into();
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&engine);
    let routed = render(&mut bypass, 480);
    let mut modulator_only = bypass_patch.clone();
    modulator_only.output = Some("mod".into());
    modulator_only.output_port = "square".into();
    let mut source = modulator_only.compile().unwrap();
    source.adopt_state_from(&engine);
    let direct = render(&mut source, 480);
    // One engine-v2 cable sample separates the source from the routed output.
    assert_eq!(
        routed[1..],
        direct[..479],
        "bypass routes modulator to aux unchanged"
    );
    assert_eq!(lights(&bypass).brightness, before.brightness);
}

#[test]
fn bypass_freezes_runtime_and_initialize_matches_fresh_reopen() {
    // A constant modulator and the built-in carrier make the inputs
    // independent of upstream history, isolating Cross Matter's own state.
    let p: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"dc","kind":"poly_constant","params":{"channels":1.0,"v1":2.0}},
            {"id":"cross","kind":"cross_matter","params":{"algorithm":0.3,"carrier":2.0,"level1":0.4}}],
        "cables":[{"from":"dc","output":"out","to":"cross","input":"modulator"}],
        "output":"cross","output_port":"out"}))
    .unwrap();
    let mut engine = p.compile().unwrap();
    render(&mut engine, 4801);
    let mut bypass_patch = p.clone();
    bypass_patch.nodes[1].bypassed = true;
    let mut bypass = bypass_patch.compile().unwrap();
    bypass.adopt_state_from(&engine);
    render(&mut bypass, 480);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypass);
    let a = render(&mut resumed, 977);
    let b = render(&mut engine, 977);
    assert_eq!(a, b, "bypass freezes runtime state");
    assert!(rms(&a) > 0.1);

    // Initialize empties the pipeline: the 33-frame output pre-roll at
    // 48 kHz is silent before the free-running built-in carrier returns.
    let handle = resumed.resolve_module_lights("cross").unwrap();
    assert!(resumed.initialize_module_state(handle));
    let a = render(&mut resumed, 400);
    assert!(a[..33].iter().all(|x| *x == 0.0));
    assert!(rms(&a[105..]) > 0.1);
}

#[test]
fn initialize_matches_a_freshly_built_module_after_any_history() {
    let node = |params: &[(&str, f32)]| Node {
        id: "cross".into(),
        kind: "cross_matter".into(),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
    };
    let input = |n: usize| {
        let mut ports = [PortFrame::default(); 6];
        ports[0] = PortFrame::mono(4.0 * (n as f32 * 0.031).sin());
        ports[1] = PortFrame::mono(3.0 * (n as f32 * 0.0173).sin());
        ports
    };
    for params in [
        &[("algorithm", 0.8), ("carrier", 3.0)][..],
        &[("mode", 1.0), ("carrier", 0.0)][..],
    ] {
        let mut used = CrossMatterProbe::new(&node(params), 48000).unwrap();
        for n in 0..9001 {
            used.step(&input(n), 0b11);
        }
        used.initialize();
        let mut fresh = CrossMatterProbe::new(&node(params), 48000).unwrap();
        for n in 0..4000 {
            let a = used.step(&input(n), 0b11);
            let b = fresh.step(&input(n), 0b11);
            assert_eq!(a, b, "frame {n}");
        }
    }
}

#[test]
fn catalog_exposes_source_contract_and_rejects_invalid_edits() {
    let modules = catalog();
    let module = modules["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "cross_matter")
        .unwrap();
    assert_eq!(module["outputs"], json!(["out", "aux"]));
    assert_eq!(
        module["inputs"],
        json!([
            "carrier",
            "modulator",
            "level1_cv",
            "level2_cv",
            "algorithm_cv",
            "timbre_cv"
        ])
    );
    assert_eq!(module["polyphony"]["maximum_channels"], 1);
    assert_eq!(
        module["params"]["carrier"]["labels"],
        json!(["External", "Sine", "Triangle", "Saw"])
    );
    assert!(module["processing_policy"]
        .as_str()
        .unwrap()
        .contains("96 kHz"));
    for (name, value) in [
        ("algorithm", -0.1),
        ("timbre", 1.5),
        ("carrier", 1.5),
        ("carrier", 4.0),
        ("mode", 2.0),
        ("unknown", 1.0),
    ] {
        let mut bad = patch(json!({}), 48000);
        bad.nodes[2].params.insert(name.into(), value);
        assert!(bad.compile().is_err(), "invalid {name}={value}");
    }
}

#[test]
fn morphing_duet_example_renders_bounded_distinct_stereo() {
    let patch: Patch = serde_json::from_str(include_str!(
        "../../../examples/cross-matter-morphing-duet.json"
    ))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    let mut audio = vec![[0.0f32; 2]; 48000 * 4];
    engine.process_audio_block(&mut audio);
    let tail = &audio[48000..];
    let peak = tail.iter().flatten().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.1 && peak < 1.0, "peak {peak}");
    let left: Vec<f32> = tail.iter().map(|f| f[0]).collect();
    let right: Vec<f32> = tail.iter().map(|f| f[1]).collect();
    assert!(rms(&left) > 0.02 && rms(&right) > 0.02);
    let difference: Vec<f32> = left.iter().zip(&right).map(|(a, b)| a - b).collect();
    assert!(rms(&difference) > 0.01, "out and aux differ");
}
