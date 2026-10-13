use openpatch_core::{catalog, ModuleParameterActions, Node, Patch};
use std::collections::BTreeMap;

fn node(kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        id: "target".into(),
        kind: kind.into(),
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ((*name).into(), *value))
            .collect(),
    }
}

fn descriptor(kind: &str) -> serde_json::Value {
    catalog()["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == kind)
        .unwrap()
        .clone()
}

#[test]
fn every_parameter_declares_initialize_and_randomization_policy() {
    let catalog = catalog();
    let mut kinds = 0;
    for module in catalog["modules"].as_array().unwrap() {
        kinds += 1;
        for spec in module["params"].as_object().unwrap().values() {
            assert!(spec["initialize"].is_boolean());
            assert!(matches!(
                spec["randomization"].as_str(),
                Some("disabled" | "linear" | "log2" | "inclusive_integer" | "enum")
            ));
        }
    }
    assert!(kinds > 30);
    assert_eq!(
        descriptor("multi_vco")["params"]["frequency"]["randomization"],
        "log2"
    );
    assert_eq!(
        descriptor("multi_vco")["params"]["fm_amount"]["randomization"],
        "disabled"
    );
    assert_eq!(
        descriptor("multi_lfo")["params"]["pwm_amount"]["randomization"],
        "disabled"
    );
    assert_eq!(descriptor("seq3")["params"]["running"]["initialize"], false);
    assert_eq!(
        descriptor("seq3")["params"]["gate1"]["randomization"],
        "enum"
    );
    for (kind, name) in [
        ("mix6", "invert"),
        ("mix6", "average"),
        ("vca_mix", "channel_exp"),
        ("vca_mix", "mix_exp"),
        ("switch_1to4", "declick"),
        ("switch_4to1", "declick"),
        ("fade", "law"),
        ("poly_merge", "channels"),
        ("rescale", "multiplier"),
        ("rescale", "reflect_min"),
        ("rescale", "reflect_max"),
    ] {
        assert_eq!(
            descriptor(kind)["params"][name]["randomization"],
            "disabled",
            "{kind}.{name}"
        );
    }
    assert_eq!(descriptor("mix6")["params"]["invert"]["initialize"], false);
    assert_eq!(descriptor("mix6")["params"]["average"]["initialize"], false);
}

#[test]
fn seq3_initialize_preserves_running_value_and_presence() {
    let actions = ModuleParameterActions::new(48_000);
    let mut implicit = node("seq3", &[("tempo", 300.0), ("row2_4", 7.0)]);
    actions.initialize(&mut implicit).unwrap();
    assert!(!implicit.params.contains_key("running"));
    assert_eq!(implicit.params["tempo"], 120.0);
    assert_eq!(implicit.params["row2_4"], 0.0);
    assert_eq!(implicit.params["gate8"], 1.0);

    let mut explicit = node("seq3", &[("running", 0.0), ("tempo", 300.0)]);
    actions.initialize(&mut explicit).unwrap();
    assert_eq!(explicit.params["running"], 0.0);
    assert_eq!(explicit.params["tempo"], 120.0);
    explicit.params.insert("running".into(), 1.0);
    actions.initialize(&mut explicit).unwrap();
    assert_eq!(explicit.params["running"], 1.0);
}

#[test]
fn seeded_randomization_is_deterministic_and_respects_disabled_controls() {
    let actions = ModuleParameterActions::new(48_000);
    let original = node(
        "multi_vco",
        &[
            ("frequency", 300.0),
            ("fm_amount", 0.7),
            ("pwm_amount", -0.4),
        ],
    );
    let mut first = original.clone();
    let mut second = original.clone();
    actions.randomize(&mut first, 1234).unwrap();
    actions.randomize(&mut second, 1234).unwrap();
    assert_eq!(first.params, second.params);
    assert_eq!(first.params["fm_amount"], 0.7);
    assert_eq!(first.params["pwm_amount"], -0.4);
    let mut changed = original;
    actions.randomize(&mut changed, 1235).unwrap();
    assert_ne!(first.params["frequency"], changed.params["frequency"]);

    let mut seq = node(
        "seq3",
        &[
            ("tempo", 321.0),
            ("tempo_amount", 0.3),
            ("steps", 3.0),
            ("steps_amount", 0.4),
            ("passthrough", 1.0),
            ("running", 0.0),
        ],
    );
    actions.randomize(&mut seq, 9).unwrap();
    for (key, value) in [
        ("tempo", 321.0),
        ("tempo_amount", 0.3),
        ("steps", 3.0),
        ("steps_amount", 0.4),
        ("passthrough", 1.0),
        ("running", 0.0),
    ] {
        assert_eq!(seq.params[key], value);
    }
    assert!((-10.0..=10.0).contains(&seq.params["row1_1"]));
    assert!(seq.params["gate1"] == 0.0 || seq.params["gate1"] == 1.0);
}

#[test]
fn custom_menu_settings_are_not_knob_randomized_and_mix6_initialize_preserves_presence() {
    let actions = ModuleParameterActions::new(48_000);
    let mut implicit = node("mix6", &[("level", 0.2)]);
    actions.initialize(&mut implicit).unwrap();
    assert_eq!(implicit.params["level"], 1.0);
    assert!(!implicit.params.contains_key("invert"));
    assert!(!implicit.params.contains_key("average"));

    let mut explicit = node("mix6", &[("invert", 1.0), ("average", 1.0)]);
    actions.initialize(&mut explicit).unwrap();
    assert_eq!(explicit.params["invert"], 1.0);
    assert_eq!(explicit.params["average"], 1.0);
    actions.randomize(&mut explicit, 17).unwrap();
    assert_eq!(explicit.params["invert"], 1.0);
    assert_eq!(explicit.params["average"], 1.0);

    for (kind, key, value) in [
        ("vca_mix", "channel_exp", 1.0),
        ("vca_mix", "mix_exp", 1.0),
        ("switch_1to4", "declick", 1.0),
        ("switch_4to1", "declick", 1.0),
        ("fade", "law", 1.0),
        ("poly_merge", "channels", 7.0),
        ("rescale", "multiplier", 100.0),
        ("rescale", "reflect_min", 1.0),
        ("rescale", "reflect_max", 1.0),
    ] {
        let mut target = node(kind, &[(key, value)]);
        actions.randomize(&mut target, 17).unwrap();
        assert_eq!(target.params[key], value, "{kind}.{key}");
        let mut implicit = node(kind, &[]);
        actions.randomize(&mut implicit, 17).unwrap();
        assert!(!implicit.params.contains_key(key), "{kind}.{key}");
    }
    for multiplier in [1.0, 10.0, 100.0, 1000.0] {
        let mut target = node("rescale", &[("multiplier", multiplier)]);
        actions.randomize(&mut target, 17).unwrap();
        assert_eq!(target.params["multiplier"], multiplier);
        actions.initialize(&mut target).unwrap();
        assert_eq!(target.params["multiplier"], 1.0);
    }
}

#[test]
fn log_integer_and_enum_policies_reach_their_intended_domains() {
    let actions = ModuleParameterActions::new(48_000);
    let mut log_sum = 0.0_f64;
    let mut steps_seen = [false; 3];
    let mut waveforms_seen = [false; 2];
    let mut gate_seen = [false; 2];
    for seed in 0..2048 {
        let mut vco = node("multi_vco", &[]);
        actions.randomize(&mut vco, seed).unwrap();
        log_sum += f64::from(vco.params["frequency"]).log2();
        let mut switch = node("switch_1to4", &[]);
        actions.randomize(&mut switch, seed).unwrap();
        steps_seen[(switch.params["steps"] as usize) - 2] = true;
        let mut analog_vco = node("analog_vco", &[]);
        actions.randomize(&mut analog_vco, seed).unwrap();
        waveforms_seen[analog_vco.params["waveform"] as usize] = true;
        let mut seq = node("seq3", &[]);
        actions.randomize(&mut seq, seed).unwrap();
        gate_seen[seq.params["gate1"] as usize] = true;
    }
    assert!(steps_seen.into_iter().all(|seen| seen));
    assert!(waveforms_seen.into_iter().all(|seen| seen));
    assert!(gate_seen.into_iter().all(|seen| seen));
    let expected = (11.562_326_f64.log2() + 5_919.911_f64.log2()) / 2.0;
    assert!((log_sum / 2048.0 - expected).abs() < 0.18);
}

#[test]
fn rate_bounds_errors_and_every_kind_remain_compilable() {
    assert!(ModuleParameterActions::new(7999)
        .initialize(&mut node("constant", &[]))
        .is_err());
    let actions = ModuleParameterActions::new(8000);
    assert!(actions.initialize(&mut node("unknown", &[])).is_err());
    assert!(actions
        .randomize(&mut node("constant", &[("invalid", 1.0)]), 1)
        .is_err());
    for kind in ["oscillator", "lowpass", "analog_vco", "ladder_vcf"] {
        for seed in 0..100 {
            let mut target = node(kind, &[]);
            actions.randomize(&mut target, seed).unwrap();
            let frequency = match kind {
                "oscillator" | "analog_vco" => target.params["frequency"],
                _ => target.params["cutoff"],
            };
            assert!(
                frequency
                    <= if kind == "oscillator" || kind == "lowpass" {
                        3600.0
                    } else {
                        3200.0
                    }
            );
            let mut patch = Patch::new_v2();
            patch.sample_rate = 8000;
            patch.nodes.push(target);
            patch.compile().unwrap();
        }
    }
    // Every descriptor's chosen policy and defaults must compile at the
    // minimum, common, and maximum supported rates.
    for module in catalog()["modules"].as_array().unwrap() {
        let kind = module["kind"].as_str().unwrap();
        for rate in [8000, 48_000, 192_000] {
            let actions = ModuleParameterActions::new(rate);
            let mut patch = Patch::new_v2();
            patch.sample_rate = rate;
            let mut initialized = node(kind, &[]);
            actions.initialize(&mut initialized).unwrap();
            patch.nodes.push(initialized);
            patch
                .compile()
                .unwrap_or_else(|error| panic!("{kind} initialize at {rate}: {error}"));
            for seed in 0..24 {
                let mut target = node(kind, &[]);
                actions.randomize(&mut target, seed).unwrap();
                patch.nodes.clear();
                patch.nodes.push(target);
                patch
                    .compile()
                    .unwrap_or_else(|error| panic!("{kind} seed {seed} at {rate}: {error}"));
            }
        }
    }
}

#[test]
fn initialize_runtime_handle_accepts_valid_bypassed_and_rejects_foreign() {
    let mut patch = Patch::new_v2();
    patch.nodes = vec![
        node("multi_vco", &[]),
        Node {
            id: "bypassed".into(),
            kind: "constant".into(),
            bypassed: true,
            params: BTreeMap::new(),
        },
    ];
    patch.output = Some("target".into());
    patch.output_port = "sine".into();
    let mut first = patch.compile().unwrap();
    first.process_poly_block(&mut [openpatch_core::PortFrame::default(); 137]);
    let mut second = patch.compile().unwrap();
    let first_vco = first.resolve_module_state("target").unwrap();
    let first_bypassed = first.resolve_module_state("bypassed").unwrap();
    assert!(first.initialize_module_state(first_vco));
    assert!(first.initialize_module_state(first_bypassed));
    assert!(!second.initialize_module_state(first_vco));

    let mut expected = patch.compile().unwrap();
    expected.adopt_state_from(&first);
    second.adopt_state_from(&first);
    let second_vco = second.resolve_module_state("target").unwrap();
    assert!(second.initialize_module_state(second_vco));
    let mut a = [openpatch_core::PortFrame::default(); 64];
    let mut b = [openpatch_core::PortFrame::default(); 64];
    expected.process_poly_block(&mut a);
    second.process_poly_block(&mut b);
    assert_eq!(a, b);
}
