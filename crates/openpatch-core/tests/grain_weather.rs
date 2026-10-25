use openpatch_core::{
    catalog, Engine, GrainWeatherProbe, ModuleLightState, Node, Patch, PortFrame,
};
use serde_json::{json, Value};

/// Two stock VCOs (saw left, square right) into Grain Weather.
fn patch(params: Value, rate: u32) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":rate,
        "nodes":[
            {"id":"a","kind":"stock_vco","params":{"frequency":-12.0}},
            {"id":"b","kind":"stock_vco","params":{"frequency":7.0}},
            {"id":"grain","kind":"grain_weather","params":params}],
        "cables":[
            {"from":"a","output":"saw","to":"grain","input":"left"},
            {"from":"b","output":"square","to":"grain","input":"right"}],
        "output":"grain","output_port":"left"}))
    .unwrap()
}

fn node(params: &[(&str, f32)]) -> Node {
    Node {
        id: "grain".into(),
        kind: "grain_weather".into(),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
    }
}

fn render(engine: &mut Engine, count: usize) -> Vec<f32> {
    let mut frames = vec![PortFrame::default(); count];
    engine.process_poly_block(&mut frames);
    frames.iter().map(|f| f.channel(0)).collect()
}

fn lights(engine: &Engine) -> ModuleLightState {
    engine
        .module_lights(engine.resolve_module_lights("grain").unwrap())
        .unwrap()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// A plucked stereo source: decaying tones retriggered every quarter second.
fn source(n: usize, rate: u32) -> [PortFrame; 10] {
    let t = n as f32 / rate as f32;
    let envelope = (-(t % 0.25) * 12.0).exp();
    let mut ports = [PortFrame::default(); 10];
    ports[0] = PortFrame::mono(5.0 * envelope * (std::f32::consts::TAU * 220.0 * t).sin());
    ports[1] = PortFrame::mono(4.0 * envelope * (std::f32::consts::TAU * 331.0 * t).sin());
    ports
}

fn run_probe(probe: &mut GrainWeatherProbe, from: usize, count: usize, rate: u32) -> Vec<[f32; 2]> {
    (from..from + count)
        .map(|n| {
            let out = probe.step(&source(n, rate), 0b11);
            [out[0].channel(0), out[1].channel(0)]
        })
        .collect()
}

#[test]
fn native_32k_host_matches_frozen_cpp_frames_after_fixed_latency() {
    // Knob settings pass to the kernel unchanged with unpatched CV inputs.
    // Volts word/4096 map to codec word `word`; output words map back exactly
    // at 16 V full scale.
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/grain-weather-host-golden.json")).unwrap();
    const LATENCY: usize = 35;
    let cases: [(&str, &[(&str, f32)]); 2] = [
        (
            "host_golden_granular",
            &[
                ("position", 0.4),
                ("size", 0.55),
                ("density", 0.7),
                ("texture", 0.6),
                ("blend", 0.8),
                ("spread", 0.6),
                ("feedback", 0.2),
                ("reverb", 0.3),
            ],
        ),
        (
            "host_golden_spectral",
            &[
                ("mode", 3.0),
                ("position", 0.25),
                ("size", 0.6),
                ("density", 0.6),
                ("texture", 0.4),
                ("blend", 1.0),
                ("reverb", 0.2),
            ],
        ),
    ];
    for (case, params) in cases {
        let mut probe = GrainWeatherProbe::new(&node(params), 32000).unwrap();
        let data = &fixture["cases"][case];
        let words = |key: &str| -> Vec<i64> {
            data[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect()
        };
        let inputs = words("inputs");
        let expected = words("outputs");
        let frames = inputs.len() / 2;
        let mut actual = Vec::with_capacity(2 * frames);
        for n in 0..frames + LATENCY {
            let mut ports = [PortFrame::default(); 10];
            let (l, r) = if n < frames {
                (inputs[2 * n], inputs[2 * n + 1])
            } else {
                (0, 0)
            };
            ports[0] = PortFrame::mono(l as f32 / 4096.0);
            ports[1] = PortFrame::mono(r as f32 / 4096.0);
            let out = probe.step(&ports, 0b11);
            if n < LATENCY {
                assert_eq!(out[0].channel(0), 0.0, "{case} pre-roll frame {n}");
                continue;
            }
            for output in out {
                let word = output.channel(0) * 2048.0;
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
        assert!(expected.iter().filter(|v| **v != 0).count() > expected.len() / 2);
        assert_eq!(probe.snapshot().guarded_reads, 0);
    }
}

#[test]
fn all_four_modes_sound_and_differ_from_each_other() {
    let renders: Vec<Vec<f32>> = (0..4)
        .map(|mode| {
            let mut probe = GrainWeatherProbe::new(
                &node(&[
                    ("mode", mode as f32),
                    ("blend", 1.0),
                    ("position", 0.1),
                    ("density", 0.8),
                    ("size", 0.4),
                ]),
                48000,
            )
            .unwrap();
            run_probe(&mut probe, 0, 96000, 48000)[48000..]
                .iter()
                .map(|f| f[0])
                .collect()
        })
        .collect();
    let levels: Vec<f32> = renders.iter().map(|r| rms(r)).collect();
    // Includes the stretch mode, silent at the pinned revision without the
    // restored Window::Start line.
    assert!(levels.iter().all(|&l| l > 0.05), "silent mode: {levels:?}");
    for a in 0..4 {
        for b in a + 1..4 {
            let difference: Vec<f32> = renders[a]
                .iter()
                .zip(&renders[b])
                .map(|(x, y)| x - y)
                .collect();
            let ratio = rms(&difference) / levels[a].max(levels[b]);
            assert!(ratio > 0.3, "modes {a} and {b} too similar: {ratio}");
        }
    }
}

#[test]
fn quality_selects_buffer_format_length_and_grain_budget() {
    // Seconds of audio the preallocated memory holds at each setting, and the
    // source grain limit (32/40 at 16 bits, 46/57 at 8 bits).
    for (quality, seconds, grains) in [
        (0.0, 1.022, 32),
        (1.0, 1.856, 40),
        (2.0, 4.088, 46),
        (3.0, 7.424, 57),
    ] {
        let mut probe = GrainWeatherProbe::new(
            &node(&[("quality", quality), ("density", 1.0), ("size", 1.0)]),
            32000,
        )
        .unwrap();
        let out = run_probe(&mut probe, 0, 32000, 32000);
        let snapshot = probe.snapshot();
        assert_eq!(snapshot.quality as f32, quality);
        assert!(
            (snapshot.buffer_seconds - seconds).abs() < 0.01,
            "{snapshot:?}"
        );
        assert!(
            snapshot.active_grains > grains / 2 && snapshot.active_grains <= grains,
            "{snapshot:?}"
        );
        assert!(rms(&out[16000..].iter().map(|f| f[0]).collect::<Vec<_>>()) > 0.05);
    }
}

#[test]
fn trigger_input_seeds_grains_when_density_is_centred() {
    let mut probe =
        GrainWeatherProbe::new(&node(&[("density", 0.5), ("size", 0.3)]), 48000).unwrap();
    run_probe(&mut probe, 0, 24000, 48000);
    assert_eq!(probe.snapshot().active_grains, 0, "no free-running grains");
    let mut grains = 0;
    for n in 24000..26400 {
        let mut ports = source(n, 48000);
        ports[2] = PortFrame::mono(if n < 24100 { 5.0 } else { 0.0 });
        probe.step(&ports, 0b11);
        grains = grains.max(probe.snapshot().active_grains);
    }
    assert_eq!(grains, 1, "one trigger, one grain");
}

#[test]
fn freeze_gate_holds_the_buffer_after_the_input_stops() {
    for mode in [0.0, 2.0, 3.0] {
        let params = [
            ("mode", mode),
            ("blend", 1.0),
            ("position", 0.2),
            ("density", 0.8),
        ];
        let mut open = GrainWeatherProbe::new(&node(&params), 48000).unwrap();
        let mut frozen = GrainWeatherProbe::new(&node(&params), 48000).unwrap();
        run_probe(&mut open, 0, 48000, 48000);
        run_probe(&mut frozen, 0, 48000, 48000);
        let mut tails = [Vec::new(), Vec::new()];
        for _ in 0..96000 {
            let mut silent = [PortFrame::default(); 10];
            silent[0] = PortFrame::mono(0.0);
            tails[0].push(open.step(&silent, 0b11)[0].channel(0));
            silent[3] = PortFrame::mono(5.0);
            tails[1].push(frozen.step(&silent, 0b11)[0].channel(0));
        }
        assert!(frozen.snapshot().frozen && !open.snapshot().frozen);
        assert_eq!(frozen.lights().brightness[2], [0.0, 1.0]);
        let late = 72000..96000;
        assert!(
            rms(&tails[0][late.clone()]) < 0.02,
            "mode {mode}: unfrozen buffer empties"
        );
        assert!(
            rms(&tails[1][late]) > 0.05,
            "mode {mode}: frozen buffer keeps playing"
        );
    }
    let mut switch = GrainWeatherProbe::new(&node(&[("freeze", 1.0)]), 48000).unwrap();
    run_probe(&mut switch, 0, 200, 48000);
    assert!(switch.snapshot().frozen, "freeze switch");
}

#[test]
fn chunk_boundaries_and_graph_adoption_are_seamless() {
    let p = patch(json!({"density":0.8,"spread":0.7,"mode":0.0}), 48000);
    let expected = render(&mut p.compile().unwrap(), 4101);
    let mut engine = p.compile().unwrap();
    let mut offset = 0;
    for count in [1, 2, 31, 0, 32, 33, 128, 3, 97, 129, 3645] {
        let samples = render(&mut engine, count);
        for (a, b) in samples.iter().zip(&expected[offset..offset + count]) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        offset += count;
        let mut replacement = p.compile().unwrap();
        replacement.adopt_state_from(&engine);
        engine = replacement;
    }
    assert_eq!(offset, 4101);
    assert!(rms(&expected[2000..]) > 0.05);
}

#[test]
fn bypass_routes_left_and_right_unchanged() {
    let p = patch(json!({"mode":3.0}), 48000);
    let mut engine = p.compile().unwrap();
    render(&mut engine, 4801);
    let before = lights(&engine);
    assert_eq!(before.count, 3);
    for (port, from, output) in [("left", "a", "saw"), ("right", "b", "square")] {
        let mut bypass_patch = p.clone();
        bypass_patch.nodes[2].bypassed = true;
        bypass_patch.output_port = port.into();
        let mut bypass = bypass_patch.compile().unwrap();
        bypass.adopt_state_from(&engine);
        let routed = render(&mut bypass, 480);
        let mut direct_patch = bypass_patch.clone();
        direct_patch.output = Some(from.into());
        direct_patch.output_port = output.into();
        let mut source = direct_patch.compile().unwrap();
        source.adopt_state_from(&engine);
        let direct = render(&mut source, 480);
        // One engine-v2 cable sample separates the source from the output.
        assert_eq!(routed[1..], direct[..479], "bypass routes {port} unchanged");
        assert_eq!(lights(&bypass).brightness, before.brightness);
    }
}

#[test]
fn initialize_matches_a_freshly_built_module_after_any_history() {
    for params in [
        &[("mode", 0.0), ("quality", 2.0), ("density", 0.9)][..],
        &[("mode", 1.0), ("pitch", 7.0)][..],
        &[("mode", 2.0), ("freeze", 1.0)][..],
        &[("mode", 3.0), ("quality", 1.0)][..],
    ] {
        let mut used = GrainWeatherProbe::new(&node(params), 48000).unwrap();
        run_probe(&mut used, 0, 30001, 48000);
        used.initialize();
        let mut fresh = GrainWeatherProbe::new(&node(params), 48000).unwrap();
        let a = run_probe(&mut used, 0, 20000, 48000);
        let b = run_probe(&mut fresh, 0, 20000, 48000);
        assert_eq!(a, b, "{params:?}");
        assert!(rms(&a[10000..].iter().map(|f| f[0]).collect::<Vec<_>>()) > 0.01);
    }
}

#[test]
fn catalog_exposes_source_contract_and_rejects_invalid_edits() {
    let modules = catalog();
    let module = modules["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "grain_weather")
        .unwrap();
    assert_eq!(module["outputs"], json!(["left", "right"]));
    assert_eq!(
        module["inputs"],
        json!([
            "left",
            "right",
            "trigger",
            "freeze",
            "position_cv",
            "size_cv",
            "pitch_cv",
            "density_cv",
            "texture_cv",
            "blend_cv"
        ])
    );
    assert_eq!(module["polyphony"]["maximum_channels"], 1);
    assert_eq!(
        module["params"]["mode"]["labels"],
        json!(["Granular", "Stretch", "Looping delay", "Spectral"])
    );
    assert_eq!(
        module["params"]["quality"]["labels"],
        json!(["16-bit stereo", "16-bit mono", "8-bit stereo", "8-bit mono"])
    );
    assert!(module["processing_policy"]
        .as_str()
        .unwrap()
        .contains("32 kHz"));
    assert!(module["state_policy"]
        .as_str()
        .unwrap()
        .contains("never buffer audio"));
    for (name, value) in [
        ("position", -0.1),
        ("pitch", 25.0),
        ("mode", 1.5),
        ("mode", 4.0),
        ("quality", 4.0),
        ("freeze", 0.5),
        ("unknown", 1.0),
    ] {
        let mut bad = patch(json!({}), 48000);
        bad.nodes[2].params.insert(name.into(), value);
        assert!(bad.compile().is_err(), "invalid {name}={value}");
    }
}

#[test]
fn saved_patches_hold_authored_controls_not_buffer_audio() {
    let p = patch(json!({"mode":2.0,"freeze":1.0,"position":0.25}), 48000);
    let mut engine = p.compile().unwrap();
    render(&mut engine, 48000);
    let saved = serde_json::to_value(&p).unwrap();
    let grain = saved["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "grain")
        .unwrap();
    assert_eq!(
        grain["params"],
        json!({"mode":2.0,"freeze":1.0,"position":0.25})
    );
    assert!(serde_json::to_string(&saved).unwrap().len() < 4096);
}

#[test]
fn rainfall_example_renders_bounded_distinct_stereo() {
    let patch: Patch = serde_json::from_str(include_str!(
        "../../../examples/grain-weather-rainfall.json"
    ))
    .unwrap();
    let mut engine = patch.compile().unwrap();
    let mut audio = vec![[0.0f32; 2]; 48000 * 4];
    engine.process_audio_block(&mut audio);
    let tail = &audio[48000..];
    let peak = tail.iter().flatten().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.05 && peak < 1.0, "peak {peak}");
    let left: Vec<f32> = tail.iter().map(|f| f[0]).collect();
    let right: Vec<f32> = tail.iter().map(|f| f[1]).collect();
    assert!(rms(&left) > 0.01 && rms(&right) > 0.01);
    let difference: Vec<f32> = left.iter().zip(&right).map(|(a, b)| a - b).collect();
    assert!(rms(&difference) > 0.005, "left and right differ");
}
