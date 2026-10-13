use openpatch_core::{demo_patch, render, write_wav, Cable, Node, Patch, MAX_CABLES, MAX_NODES};
use std::collections::BTreeMap;

fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        bypassed: false,
        id: id.into(),
        kind: kind.into(),
        params: params
            .iter()
            .map(|(key, value)| (key.to_string(), *value))
            .collect(),
    }
}

fn cable(from: &str, to: &str, input: &str) -> Cable {
    Cable {
        from: from.into(),
        to: to.into(),
        input: input.into(),
        output: "out".into(),
    }
}

fn oscillator(frequency: f32) -> Patch {
    Patch {
        nodes: vec![node(
            "osc",
            "oscillator",
            &[("frequency", frequency), ("amplitude", 0.75)],
        )],
        output: Some("osc".into()),
        ..Patch::default()
    }
}

#[test]
fn oscillator_has_expected_frequency_amplitude_and_dc() {
    let samples = render(&oscillator(440.0), 1.0).unwrap();
    let crossings = samples
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    assert!(
        (439..=440).contains(&crossings),
        "positive crossings: {crossings}"
    );
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, value| peak.max(value.abs()));
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let rms = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
    assert!((peak - 0.75).abs() < 0.0001);
    assert!(mean.abs() < 0.00001);
    assert!((rms - 0.75 / 2.0f32.sqrt()).abs() < 0.0001);
}

#[test]
fn node_and_cable_order_do_not_change_render() {
    let patch = demo_patch();
    let expected = render(&patch, 0.5).unwrap();
    let mut reordered = patch.clone();
    reordered.nodes.reverse();
    reordered.cables.reverse();
    assert_eq!(expected, render(&reordered, 0.5).unwrap());
}

#[test]
fn arbitrary_callback_sizes_produce_identical_samples() {
    let patch = demo_patch();
    let mut reference = patch.compile().unwrap();
    let mut engine = patch.compile().unwrap();
    let mut expected = vec![0.0; 10_017];
    let mut actual = vec![0.0; expected.len()];
    reference.process_block(&mut expected);
    engine.process_block(&mut []);
    let sizes = [1, 7, 128, 257, 3, 64, 512, 129];
    let mut offset = 0;
    let mut cursor = 0;
    while offset < actual.len() {
        let end = (offset + sizes[cursor % sizes.len()]).min(actual.len());
        engine.process_block(&mut actual[offset..end]);
        offset = end;
        cursor += 1;
    }
    assert_eq!(expected, actual);
}

#[test]
fn fm_and_vca_apply_modulation_to_each_sample() {
    let mut patch = oscillator(100.0);
    patch.nodes.push(node("fm", "constant", &[("value", 16.0)]));
    patch.cables.push(cable("fm", "osc", "fm"));
    assert_eq!(
        render(&patch, 0.1).unwrap(),
        render(&oscillator(116.0), 0.1).unwrap()
    );

    let mut patch = demo_patch();
    patch.output = Some("vca".into());
    let mut tone_patch = patch.clone();
    tone_patch.output = Some("tone".into());
    let mut lfo_patch = patch.clone();
    lfo_patch.output = Some("tremolo".into());
    let vca = render(&patch, 0.1).unwrap();
    let tone = render(&tone_patch, 0.1).unwrap();
    let lfo = render(&lfo_patch, 0.1).unwrap();
    for i in 0..vca.len() {
        assert_eq!(vca[i], tone[i] * lfo[i]);
    }
}

#[test]
fn lowpass_step_matches_one_pole_response_and_attenuates_highs() {
    let patch = Patch {
        nodes: vec![
            node("step", "constant", &[("value", 1.0)]),
            node("lp", "lowpass", &[("cutoff", 1000.0)]),
        ],
        cables: vec![cable("step", "lp", "signal")],
        output: Some("lp".into()),
        ..Patch::default()
    };
    let samples = render(&patch, 0.02).unwrap();
    let alpha = 1.0 - (-std::f64::consts::TAU * 1000.0 / 48000.0).exp();
    for (i, actual) in samples.iter().take(30).enumerate() {
        let expected = 1.0 - (1.0 - alpha).powi(i as i32 + 1);
        assert!((*actual as f64 - expected).abs() < 0.000001);
    }
    assert!((samples.last().unwrap() - 1.0).abs() < 0.000001);
    let filtered_rms = |frequency| {
        let mut patch = oscillator(frequency);
        patch
            .nodes
            .push(node("lp", "lowpass", &[("cutoff", 1000.0)]));
        patch.cables.push(cable("osc", "lp", "signal"));
        patch.output = Some("lp".into());
        let samples = render(&patch, 0.2).unwrap();
        (samples[4800..].iter().map(|s| s * s).sum::<f32>() / 4800.0).sqrt()
    };
    assert!(filtered_rms(8000.0) < filtered_rms(100.0) * 0.15);
}

#[test]
fn mixer_gain_unpatched_inputs_and_silent_patch() {
    let mut patch = Patch {
        nodes: vec![
            node("a", "constant", &[("value", 2.0)]),
            node("b", "constant", &[("value", 3.0)]),
            node("sum", "mixer", &[]),
            node("g", "gain", &[("gain", -0.5)]),
        ],
        cables: vec![
            cable("a", "sum", "a"),
            cable("b", "sum", "b"),
            cable("sum", "g", "signal"),
        ],
        output: Some("g".into()),
        ..Patch::default()
    };
    assert!(render(&patch, 0.01).unwrap().iter().all(|&v| v == -2.5));
    patch.cables.clear();
    assert!(render(&patch, 0.01).unwrap().iter().all(|&v| v == 0.0));
    assert!(render(&Patch::default(), 0.01)
        .unwrap()
        .iter()
        .all(|&v| v == 0.0));
}

#[test]
fn invalid_patch_data_is_rejected_with_context() {
    let mut patch = oscillator(440.0);
    patch.nodes[0].params.insert("frequency".into(), f32::NAN);
    assert!(patch.validate().unwrap_err().contains("frequency"));
    patch.nodes[0]
        .params
        .insert("frequency".into(), f32::INFINITY);
    assert!(patch.validate().is_err());
    patch.nodes[0].params.insert("frequency".into(), -1.0);
    assert!(patch.validate().is_err());
    patch.nodes[0].params.insert("frequency".into(), 440.0);
    patch.nodes[0].params.insert("typo".into(), 1.0);
    assert!(patch.validate().unwrap_err().contains("unknown parameter"));
    patch.nodes[0].params = BTreeMap::new();
    patch.nodes[0].kind = "missing".into();
    assert!(patch.validate().unwrap_err().contains("unknown module"));
    patch.nodes[0].kind = "oscillator".into();
    patch.nodes[0].id = "a/b".into();
    assert!(patch.validate().is_err());
    assert!(serde_json::from_str::<Patch>(r#"{"unexpected":true}"#).is_err());
    let patch: Patch = serde_json::from_str("{}").unwrap();
    assert_eq!(patch.version, 1);
    assert_eq!(patch.sample_rate, 48000);
    patch.validate().unwrap();
}

#[test]
fn invalid_connections_duplicate_ids_and_cycles_are_rejected() {
    let base = Patch {
        nodes: vec![node("a", "gain", &[]), node("b", "gain", &[])],
        output: Some("b".into()),
        ..Patch::default()
    };
    let mut patch = base.clone();
    patch.cables.push(cable("a", "b", "typo"));
    assert!(patch.validate().unwrap_err().contains("unknown input"));
    patch.cables = vec![cable("missing", "b", "signal")];
    assert!(patch.validate().unwrap_err().contains("source"));
    patch.cables = vec![cable("a", "missing", "signal")];
    assert!(patch.validate().unwrap_err().contains("destination"));
    patch.cables = vec![cable("a", "b", "signal"), cable("a", "b", "signal")];
    assert!(patch.validate().unwrap_err().contains("already has"));
    patch.cables = vec![cable("a", "b", "signal"), cable("b", "a", "signal")];
    assert!(patch.validate().unwrap_err().contains("cycle"));
    patch.cables = vec![cable("a", "a", "signal")];
    assert!(patch.validate().unwrap_err().contains("cycle"));
    patch.cables.clear();
    patch.nodes[1].id = "a".into();
    assert!(patch.validate().unwrap_err().contains("duplicate"));
    let mut patch = base;
    patch.output = Some("missing".into());
    assert!(patch.validate().unwrap_err().contains("output"));
}

#[test]
fn limits_versions_rates_and_render_duration_are_enforced() {
    let mut patch = Patch {
        version: 3,
        ..Patch::default()
    };
    assert!(patch.validate().is_err());
    patch.version = 1;
    for rate in [0, 7999, 192001, u32::MAX] {
        patch.sample_rate = rate;
        assert!(patch.validate().is_err());
    }
    for rate in [8000, 44100, 48000, 192000] {
        patch.sample_rate = rate;
        patch.validate().unwrap();
    }
    patch.sample_rate = 8000;
    patch.nodes = vec![node("osc", "oscillator", &[("frequency", 4000.0)])];
    assert!(patch.validate().is_err());
    patch = Patch::default();
    patch.nodes = (0..=MAX_NODES)
        .map(|i| node(&format!("n{i}"), "constant", &[]))
        .collect();
    assert!(patch.validate().unwrap_err().contains("nodes"));
    patch.nodes.clear();
    patch.cables = vec![cable("a", "b", "signal"); MAX_CABLES + 1];
    assert!(patch.validate().unwrap_err().contains("cables"));
    for duration in [-0.1, 30.001, f64::NAN, f64::INFINITY] {
        assert!(render(&Patch::default(), duration).is_err());
    }
    assert!(render(&Patch::default(), 0.0).unwrap().is_empty());
}

#[test]
fn high_gain_chain_remains_finite() {
    let mut patch = oscillator(440.0);
    for i in 0..200 {
        let id = format!("gain{i}");
        patch.nodes.push(node(&id, "gain", &[("gain", 16.0)]));
        patch
            .cables
            .push(cable(patch.output.as_ref().unwrap(), &id, "signal"));
        patch.output = Some(id);
    }
    assert!(render(&patch, 0.1)
        .unwrap()
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 1_000_000.0));
}

#[test]
fn wav_has_correct_header_clipping_and_never_overwrites() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "openpatch-test-{}-{unique}.wav",
        std::process::id()
    ));
    write_wav(&path, &[-2.0, -1.0, 0.0, 1.0, 2.0, f32::NAN], 48000).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 48);
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 48000);
    assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 12);
    let pcm: Vec<i16> = bytes[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    assert_eq!(pcm, [-32768, -32768, 0, 32767, 32767, 0]);
    assert_eq!(
        write_wav(&path, &[0.5], 48000).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    std::fs::remove_file(path).unwrap();
}
