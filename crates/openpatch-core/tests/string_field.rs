use openpatch_core::{catalog, Engine, ModuleLightState, Node, Patch, PortFrame, StringFieldProbe};
use serde_json::{json, Value};

/// A stock LFO square clock strums String Field (internal exciter).
fn patch(params: Value, rate: u32) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":rate,
        "nodes":[
            {"id":"clock","kind":"stock_lfo","params":{"rate":2.0,"offset":0.0}},
            {"id":"strings","kind":"string_field","params":params}],
        "cables":[
            {"from":"clock","output":"square","to":"strings","input":"strum"}],
        "output":"strings","output_port":"odd"}))
    .unwrap()
}

fn node(params: &[(&str, f32)]) -> Node {
    Node {
        id: "strings".into(),
        kind: "string_field".into(),
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
        .module_lights(engine.resolve_module_lights("strings").unwrap())
        .unwrap()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// Strum pulses of 5 ms every `period` frames on the STRUM input.
fn strum_ports(n: usize, period: usize) -> [PortFrame; 8] {
    let mut ports = [PortFrame::default(); 8];
    ports[1] = PortFrame::mono(if n % period < 240 { 5.0 } else { 0.0 });
    ports
}

fn run_probe(
    probe: &mut StringFieldProbe,
    count: usize,
    ports: impl Fn(usize) -> [PortFrame; 8],
) -> Vec<[f32; 2]> {
    (0..count)
        .map(|n| {
            let out = probe.step(&ports(n), 0b11);
            [out[0].channel(0), out[1].channel(0)]
        })
        .collect()
}

#[test]
fn native_48k_host_matches_frozen_cpp_frames_after_fixed_latency() {
    // Fixed knobs with only IN patched (strum and V/OCT normalized) produce
    // exactly the kernel inputs of the frozen cases. Volts word x 5/32768 map
    // to codec word `word`; output words map back exactly at 5 V full scale.
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/string-field-host-golden.json")).unwrap();
    const LATENCY: usize = 27;
    let knobs = [
        ("frequency", 30.0),
        ("structure", 0.4),
        ("brightness", 0.6),
        ("damping", 0.7),
        ("position", 0.3),
    ];
    let cases: [(&str, &[(&str, f32)]); 2] = [
        ("host_golden_modal", &[("model", 0.0), ("polyphony", 1.0)]),
        ("host_golden_string", &[("model", 2.0), ("polyphony", 2.0)]),
    ];
    for (case, extra) in cases {
        let params: Vec<(&str, f32)> = knobs.iter().chain(extra.iter()).copied().collect();
        let mut probe = StringFieldProbe::new(&node(&params), 48000).unwrap();
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
        let frames = inputs.len();
        let mut actual = Vec::with_capacity(2 * frames);
        for n in 0..frames + LATENCY {
            let mut ports = [PortFrame::default(); 8];
            let word = inputs.get(n).copied().unwrap_or(0);
            ports[0] = PortFrame::mono(word as f32 * 5.0 / 32768.0);
            let out = probe.step(&ports, 0b11);
            if n < LATENCY {
                assert_eq!(out[0].channel(0), 0.0, "{case} pre-roll frame {n}");
                continue;
            }
            for output in out {
                let word = output.channel(0) * 32768.0 / 5.0;
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
        assert!(expected.iter().filter(|v| **v != 0).count() > expected.len() / 4);
        let snapshot = probe.snapshot();
        assert_eq!(snapshot.guarded_reads, 0);
        assert!(snapshot.strums >= 3, "onset strums: {snapshot:?}");
        assert_eq!(snapshot.chord, 4);
    }
}

#[test]
fn all_six_models_sound_and_differ_from_each_other() {
    let renders: Vec<Vec<f32>> = (0..6)
        .map(|model| {
            let mut probe = StringFieldProbe::new(
                &node(&[
                    ("model", model as f32),
                    ("damping", 0.6),
                    ("brightness", 0.6),
                ]),
                48000,
            )
            .unwrap();
            run_probe(&mut probe, 48000, |n| strum_ports(n, 12000))
                .iter()
                .map(|f| f[0] + f[1])
                .collect()
        })
        .collect();
    let levels: Vec<f32> = renders.iter().map(|r| rms(r)).collect();
    assert!(levels.iter().all(|&l| l > 0.05), "silent model: {levels:?}");
    assert!(
        renders.iter().flatten().all(|x| x.abs() < 10.0),
        "outputs stay inside +/-10 V"
    );
    for a in 0..6 {
        for b in a + 1..6 {
            let difference: Vec<f32> = renders[a]
                .iter()
                .zip(&renders[b])
                .map(|(x, y)| x - y)
                .collect();
            let ratio = rms(&difference) / levels[a].max(levels[b]);
            assert!(ratio > 0.3, "models {a} and {b} too similar: {ratio}");
        }
    }
}

#[test]
fn each_strum_moves_to_the_next_voice_and_voices_split_odd_and_even() {
    for (polyphony, pattern) in [
        (1.0, &[0, 0, 0, 0][..]),
        (2.0, &[1, 0, 1, 0][..]),
        (3.0, &[1, 0, 2, 1][..]),
        (4.0, &[1, 2, 3, 0][..]),
    ] {
        let mut probe =
            StringFieldProbe::new(&node(&[("polyphony", polyphony), ("model", 2.0)]), 48000)
                .unwrap();
        let mut voices = Vec::new();
        for strum in 0..4 {
            run_probe(&mut probe, 4800, |n| strum_ports(n + strum * 4800, 4800));
            voices.push(probe.snapshot().active_voice);
        }
        assert_eq!(voices, pattern, "polyphony {polyphony}");
        assert_eq!(probe.snapshot().polyphony as f32, polyphony);
    }
    // Two voices: the first strum plays voice 1 (even output), the second
    // voice 0 (odd output).
    let mut probe =
        StringFieldProbe::new(&node(&[("polyphony", 2.0), ("model", 0.0)]), 48000).unwrap();
    let first = run_probe(&mut probe, 6000, |n| strum_ports(n, 12000));
    let odd: Vec<f32> = first.iter().map(|f| f[0]).collect();
    let even: Vec<f32> = first.iter().map(|f| f[1]).collect();
    assert!(rms(&even) > 10.0 * rms(&odd), "voice 1 on EVEN");
    let second = run_probe(&mut probe, 6000, |n| strum_ports(n + 12000, 12000));
    let odd: Vec<f32> = second.iter().map(|f| f[0]).collect();
    assert!(rms(&odd) > 0.05, "voice 0 on ODD");
}

#[test]
fn unpatched_jacks_select_the_source_strum_sources() {
    // Nothing patched: the source never strums.
    let mut idle = StringFieldProbe::new(&node(&[("model", 2.0)]), 48000).unwrap();
    let out = run_probe(&mut idle, 24000, |_| [PortFrame::default(); 8]);
    assert!(out.iter().all(|f| f[0] == 0.0 && f[1] == 0.0));
    assert_eq!(idle.snapshot().strums, 0);

    // V/OCT patched, STRUM unpatched: steps above 0.4 semitone strum.
    let mut notes = StringFieldProbe::new(&node(&[("model", 2.0)]), 48000).unwrap();
    let out = run_probe(&mut notes, 48000, |n| {
        let mut ports = [PortFrame::default(); 8];
        let step = (n / 9600) as f32;
        // Whole-semitone steps, with 0.25-semitone wobble in between.
        let wobble = if (n / 2400) % 2 == 1 {
            0.25 / 12.0
        } else {
            0.0
        };
        ports[2] = PortFrame::mono(step / 12.0 + wobble);
        ports
    });
    // The source's previous note starts at 69, so the first block also
    // strums; then one strum per semitone step, none for the wobble.
    assert_eq!(notes.snapshot().strums, 5, "one strum per semitone step");
    assert!(rms(&out.iter().map(|f| f[0]).collect::<Vec<_>>()) > 0.02);

    // IN patched, STRUM and V/OCT unpatched: input onsets strum.
    let mut onsets = StringFieldProbe::new(&node(&[("model", 0.0)]), 48000).unwrap();
    run_probe(&mut onsets, 48000, |n| {
        let mut ports = [PortFrame::default(); 8];
        let age = (n % 9600) as f32 / 48000.0;
        let pluck = (-age * 300.0).exp() * (std::f32::consts::TAU * 330.0 * age).sin();
        ports[0] = PortFrame::mono(3.0 * pluck);
        ports
    });
    let strums = onsets.snapshot().strums;
    assert!((3..=6).contains(&strums), "onset strums {strums}");
}

#[test]
fn a_single_patched_output_carries_the_sum() {
    let ports = |n: usize| strum_ports(n, 9600);
    let mut both = StringFieldProbe::new(&node(&[("model", 2.0)]), 48000).unwrap();
    let mut odd_only = StringFieldProbe::new(&node(&[("model", 2.0)]), 48000).unwrap();
    let mut even_only = StringFieldProbe::new(&node(&[("model", 2.0)]), 48000).unwrap();
    for n in 0..20000 {
        let a = both.step(&ports(n), 0b11);
        let b = odd_only.step(&ports(n), 0b01);
        let c = even_only.step(&ports(n), 0b10);
        let sum = a[0].channel(0) + a[1].channel(0);
        assert_eq!(b[0].channel(0), sum);
        assert_eq!(b[1].channels, 0);
        assert_eq!(c[1].channel(0), sum);
        assert_eq!(c[0].channels, 0);
    }
}

#[test]
fn chunk_boundaries_and_graph_adoption_are_seamless() {
    let p = patch(json!({"model":5.0,"polyphony":2.0}), 48000);
    let expected = render(&mut p.compile().unwrap(), 30101);
    let mut engine = p.compile().unwrap();
    let mut offset = 0;
    for count in [1, 2, 23, 0, 24, 25, 128, 3, 97, 129, 29669] {
        let samples = render(&mut engine, count);
        for (a, b) in samples.iter().zip(&expected[offset..offset + count]) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
        offset += count;
        let mut replacement = p.compile().unwrap();
        replacement.adopt_state_from(&engine);
        engine = replacement;
    }
    assert_eq!(offset, 30101);
    assert!(rms(&expected[2000..]) > 0.05);
}

#[test]
fn other_host_rates_resample_to_the_native_kernel() {
    for rate in [44100, 96000] {
        let mut probe = StringFieldProbe::new(&node(&[("model", 2.0)]), rate).unwrap();
        let out = run_probe(&mut probe, rate as usize, |n| {
            strum_ports(n, rate as usize / 4)
        });
        let odd: Vec<f32> = out.iter().map(|f| f[0]).collect();
        assert!(rms(&odd) > 0.05, "rate {rate}");
        assert!(odd.iter().all(|x| x.abs() < 6.0), "rate {rate}");
        assert_eq!(probe.snapshot().strums, 4, "rate {rate}");
    }
}

#[test]
fn bypass_routes_the_exciter_to_both_outputs() {
    let mut p = patch(json!({}), 48000);
    p.nodes.insert(
        0,
        serde_json::from_value(json!({"id":"tone","kind":"stock_vco","params":{"frequency":0.0}}))
            .unwrap(),
    );
    p.cables.push(
        serde_json::from_value(json!({"from":"tone","output":"sine","to":"strings","input":"in"}))
            .unwrap(),
    );
    let mut engine = p.compile().unwrap();
    render(&mut engine, 4801);
    assert_eq!(lights(&engine).count, 3);
    for port in ["odd", "even"] {
        let mut bypass_patch = p.clone();
        bypass_patch.nodes[2].bypassed = true;
        bypass_patch.output_port = port.into();
        let mut bypass = bypass_patch.compile().unwrap();
        bypass.adopt_state_from(&engine);
        let routed = render(&mut bypass, 480);
        let mut direct_patch = bypass_patch.clone();
        direct_patch.output = Some("tone".into());
        direct_patch.output_port = "sine".into();
        let mut source = direct_patch.compile().unwrap();
        source.adopt_state_from(&engine);
        let direct = render(&mut source, 480);
        // One engine-v2 cable sample separates the source from the output.
        assert_eq!(routed[1..], direct[..479], "bypass routes in to {port}");
    }
}

#[test]
fn initialize_matches_a_freshly_built_module_after_any_history() {
    for params in [
        &[("model", 0.0), ("polyphony", 4.0)][..],
        &[("model", 1.0), ("structure", 0.8)][..],
        &[("model", 2.0), ("polyphony", 3.0), ("damping", 0.98)][..],
        &[("model", 3.0), ("position", 0.0)][..],
        &[("model", 4.0), ("polyphony", 2.0)][..],
        &[("model", 5.0)][..],
    ] {
        let ports = |n: usize| strum_ports(n, 7000);
        let mut used = StringFieldProbe::new(&node(params), 48000).unwrap();
        run_probe(&mut used, 30001, ports);
        used.initialize();
        assert_eq!(used.lights().brightness[2], [0.0, 0.0]);
        let mut fresh = StringFieldProbe::new(&node(params), 48000).unwrap();
        let a = run_probe(&mut used, 20000, ports);
        let b = run_probe(&mut fresh, 20000, ports);
        assert_eq!(a, b, "{params:?}");
        assert!(rms(&a.iter().map(|f| f[0] + f[1]).collect::<Vec<_>>()) > 0.01);
    }
}

#[test]
fn strum_lamp_and_output_lamps_follow_actual_state() {
    let mut probe = StringFieldProbe::new(&node(&[("model", 2.0)]), 48000).unwrap();
    run_probe(&mut probe, 200, |n| strum_ports(n, 48000));
    assert_eq!(probe.lights().brightness[2], [0.0, 1.0], "strum lamp lit");
    run_probe(&mut probe, 4000, |n| strum_ports(n + 200, 48000));
    let lights = probe.lights();
    assert_eq!(lights.brightness[2], [0.0, 0.0], "strum lamp released");
    assert!(lights.brightness[..2].iter().flatten().any(|v| *v > 0.0));
}

#[test]
fn catalog_exposes_source_contract_and_rejects_invalid_edits() {
    let modules = catalog();
    let module = modules["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "string_field")
        .unwrap();
    assert_eq!(module["outputs"], json!(["odd", "even"]));
    assert_eq!(
        module["inputs"],
        json!([
            "in",
            "strum",
            "v_oct",
            "frequency_cv",
            "structure_cv",
            "brightness_cv",
            "damping_cv",
            "position_cv"
        ])
    );
    assert_eq!(module["polyphony"]["maximum_channels"], 1);
    assert_eq!(
        module["params"]["model"]["labels"],
        json!([
            "Modal",
            "Sympathetic strings",
            "Inharmonic string",
            "FM voice",
            "Chord strings",
            "String and reverb"
        ])
    );
    assert_eq!(module["params"]["polyphony"]["values"], json!([1, 2, 3, 4]));
    assert!(module["processing_policy"]
        .as_str()
        .unwrap()
        .contains("48 kHz"));
    assert!(module["normalization_policy"]
        .as_str()
        .unwrap()
        .contains("internal exciter"));
    for (name, value) in [
        ("structure", 1.1),
        ("frequency", 61.0),
        ("frequency_amount", -1.5),
        ("polyphony", 0.0),
        ("polyphony", 2.5),
        ("model", 6.0),
        ("unknown", 1.0),
    ] {
        let mut bad = patch(json!({}), 48000);
        bad.nodes[1].params.insert(name.into(), value);
        assert!(bad.compile().is_err(), "invalid {name}={value}");
    }
}

#[test]
fn saved_patches_hold_authored_controls_not_string_memory() {
    let p = patch(json!({"model":2.0,"damping":1.0,"position":0.25}), 48000);
    let mut engine = p.compile().unwrap();
    render(&mut engine, 48000);
    let saved = serde_json::to_value(&p).unwrap();
    let strings = saved["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == "strings")
        .unwrap();
    assert_eq!(
        strings["params"],
        json!({"model":2.0,"damping":1.0,"position":0.25})
    );
    assert!(serde_json::to_string(&saved).unwrap().len() < 4096);
}

#[test]
fn chord_strings_example_renders_bounded_distinct_stereo() {
    let patch: Patch = serde_json::from_str(include_str!(
        "../../../examples/string-field-chord-strings.json"
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
