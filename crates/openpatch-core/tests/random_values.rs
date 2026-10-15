use openpatch_core::*;
use serde_json::json;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"r","kind":"random_values"}],"output":"r","output_port":"out1"})).unwrap()
}
fn sample(engine: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    engine.process_poly_block(&mut frames);
    frames[0]
}
fn saved(engine: &Engine) -> SavedModuleState {
    engine
        .module_saved_state(engine.resolve_module_state("r").unwrap())
        .unwrap()
}
#[test]
fn catalog_ports_validation_and_parameter_action_policy() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "random_values")
        .unwrap();
    assert_eq!(m["inputs"], json!(["trig"]));
    assert_eq!(
        m["outputs"],
        json!(["out1", "out2", "out3", "out4", "out5", "out6", "out7"])
    );
    assert_eq!(m["runtime_actions"], json!(["push_press", "push_release"]));
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"].as_object().unwrap().len(), 116);
    for descriptor in m["output_metadata"].as_array().unwrap() {
        assert_eq!(descriptor["maximum_channels"], 16);
    }
    assert_eq!(m["params"]["value7_16"]["hidden"], true);
    assert_eq!(m["params"]["value7_16"]["initialize"], false);
    let mut p = patch();
    p.version = 1;
    assert!(p.compile().is_err());
    for (name, value) in [
        ("range", -1.),
        ("range", 6.),
        ("range", 0.1),
        ("seed", -1.),
        ("seed", 16_777_216.),
        ("seed", 0.5),
        ("push", 1.),
        ("value01_1", 1.),
        ("value1_17", 1.),
        ("rng_lo", 1.),
    ] {
        let mut p = patch();
        p.nodes[0].params.insert(name.into(), value);
        assert!(p.compile().is_err(), "{name}={value}");
    }
    for port in ["out", "out0", "out8", "out01"] {
        let mut p = patch();
        p.output_port = port.into();
        assert!(p.compile().is_err());
    }
    let mut n = patch().nodes.remove(0);
    let actions = ModuleParameterActions::new(48000);
    let initial = initial_random_values_state(&n).unwrap();
    actions.randomize(&mut n, 44).unwrap();
    assert_eq!(initial_random_values_state(&n).unwrap(), initial);
    actions.initialize(&mut n).unwrap();
    assert_eq!(n.params.len(), 2);
}
#[test]
fn saved_state_reroll_restore_kind_safety_and_exact_rng_resume() {
    let p = patch();
    let mut a = p.compile().unwrap();
    let h = a.resolve_module_control("r").unwrap();
    let initial = saved(&a);
    assert_eq!(SavedModuleState::from_node(&p.nodes[0]), Some(initial));
    assert!(a.handle_module_event(h, ModuleAction::PushPress));
    assert!(a.handle_module_event(h, ModuleAction::PushRelease));
    sample(&mut a);
    let rolled = saved(&a);
    assert_ne!(initial, rolled);
    let mut b = p.compile().unwrap();
    let hb = b.resolve_module_state("r").unwrap();
    assert!(b.set_module_saved_state(hb, rolled));
    assert_eq!(sample(&mut a), sample(&mut b));
    for e in [&mut a, &mut b] {
        let handle = e.resolve_module_control("r").unwrap();
        e.handle_module_event(handle, ModuleAction::PushPress);
        e.handle_module_event(handle, ModuleAction::PushRelease);
        sample(e);
    }
    assert_eq!(saved(&a), saved(&b));
    assert!(!b.set_module_saved_state(hb, SavedModuleState::PushHold(true)));
    assert!(a.set_module_saved_state(h, initial));
    assert_eq!(saved(&a), initial);
    assert!(a.initialize_module_state(h));
    assert_eq!(saved(&a), initial);
    let all = randomize_random_values_state(&p.nodes[0], initial).unwrap();
    let SavedModuleState::RandomValues { values: before, .. } = initial else {
        panic!()
    };
    let SavedModuleState::RandomValues { values: after, .. } = all else {
        panic!()
    };
    assert!(before
        .iter()
        .zip(after)
        .all(|(a, b)| a.iter().zip(b).all(|(a, b)| *a != b)));
    assert!(a
        .resolve_module_action("r", ModuleAction::HoldPress)
        .is_err());
    assert!(!a.handle_module_event(h, ModuleAction::Clock));
}
#[test]
fn adoption_pending_actions_bypass_release_and_rate_freshness() {
    let p = patch();
    let mut a = p.compile().unwrap();
    let h = a.resolve_module_control("r").unwrap();
    a.handle_module_event(h, ModuleAction::PushPress);
    sample(&mut a);
    let held = saved(&a);
    a.handle_module_event(h, ModuleAction::PushRelease);
    a.handle_module_event(h, ModuleAction::PushPress);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&a);
    assert_eq!(saved(&same), held);
    assert!(
        same.module_control_state(same.resolve_module_control("r").unwrap())
            .unwrap()
            .pressed
    );
    sample(&mut same);
    assert_eq!(saved(&same), held);
    let mut bp = p.clone();
    bp.nodes[0].bypassed = true;
    let mut bypassed = bp.compile().unwrap();
    bypassed.adopt_state_from(&a);
    let hb = bypassed.resolve_module_control("r").unwrap();
    assert!(!bypassed.module_control_state(hb).unwrap().pressed);
    assert!(bypassed
        .resolve_module_action("r", ModuleAction::PushPress)
        .is_err());
    assert!(!bypassed.handle_module_event(hb, ModuleAction::PushPress));
    assert!(bypassed.handle_module_event(hb, ModuleAction::PushRelease));
    for _ in 0..20 {
        assert_eq!(sample(&mut bypassed), PortFrame::mono(0.));
    }
    assert_eq!(saved(&bypassed), held);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    assert_eq!(saved(&resumed), held);
    let hr = resumed.resolve_module_state("r").unwrap();
    assert!(resumed.reset_module_state(hr));
    assert_eq!(
        saved(&resumed),
        initial_random_values_state(&p.nodes[0]).unwrap()
    );
    let mut different = p.clone();
    different.sample_rate = 96000;
    let mut fresh = different.compile().unwrap();
    fresh.adopt_state_from(&a);
    assert_eq!(
        saved(&fresh),
        initial_random_values_state(&p.nodes[0]).unwrap()
    );
}
#[test]
fn every_output_routes_all_sixteen_lanes_and_telemetry_reports_active_count() {
    for output in 1..=7 {
        let mut p = patch();
        p.output_port = format!("out{output}");
        p.nodes.insert(
            0,
            serde_json::from_value(
                json!({"id":"src","kind":"poly_constant","params":{"channels":16}}),
            )
            .unwrap(),
        );
        for lane in 1..=16 {
            p.nodes[0].params.insert(format!("v{lane}"), 0.);
        }
        p.cables
            .push(serde_json::from_value(json!({"from":"src","to":"r","input":"trig"})).unwrap());
        let initial = initial_random_values_state(&p.nodes[1]).unwrap();
        let SavedModuleState::RandomValues { values, .. } = initial else {
            panic!()
        };
        let mut e = p.compile().unwrap();
        let hl = e.resolve_module_lights("r").unwrap();
        assert_eq!(e.module_lights(hl).unwrap().input_channels, Some(1));
        sample(&mut e);
        let frame = sample(&mut e);
        assert_eq!(frame.channels, 16);
        assert_eq!(frame.values, values[output - 1].map(f32::from_bits));
        assert_eq!(e.module_lights(hl).unwrap().input_channels, Some(16));
    }
}

#[test]
fn serialized_snapshot_preserves_all_hidden_voltages_and_random_stream() {
    let mut p = patch();
    let mut live = p.compile().unwrap();
    let h = live.resolve_module_control("r").unwrap();
    live.handle_module_event(h, ModuleAction::PushPress);
    live.handle_module_event(h, ModuleAction::PushRelease);
    sample(&mut live);
    let full = randomize_random_values_state(&p.nodes[0], saved(&live)).unwrap();
    live.set_module_saved_state(h, full);
    full.write_params(&mut p.nodes[0].params);
    p.nodes[0].params.insert("range".into(), 2.);
    let encoded = serde_json::to_vec(&p).unwrap();
    let roundtrip: Patch = serde_json::from_slice(&encoded).unwrap();
    let mut loaded = roundtrip.compile().unwrap();
    assert_eq!(saved(&loaded), full);
    assert_eq!(saved(&live), full);
    // The range changes only subsequent draws; hidden values outside 0..1 survive.
    let SavedModuleState::RandomValues { values, .. } = full else {
        panic!()
    };
    assert!(values.iter().flatten().any(|v| f32::from_bits(*v) > 1.));
    let mut range_updated = roundtrip.compile().unwrap();
    range_updated.adopt_state_from(&live);
    for e in [&mut loaded, &mut range_updated] {
        let h = e.resolve_module_control("r").unwrap();
        e.handle_module_event(h, ModuleAction::PushPress);
        e.handle_module_event(h, ModuleAction::PushRelease);
        sample(e);
    }
    assert_eq!(saved(&loaded), saved(&range_updated));
}

#[test]
fn explicit_reroll_uses_supplied_rng_independently_of_previous_held_values() {
    let node = patch().nodes.remove(0);
    let one = SavedModuleState::RandomValues {
        values: [[1f32.to_bits(); 16]; 7],
        rng: 9987,
    };
    let two = SavedModuleState::RandomValues {
        values: [[(-9f32).to_bits(); 16]; 7],
        rng: 9987,
    };
    // A host seeded action replaces only RNG; every stored voltage is overwritten.
    let first = randomize_random_values_state(&node, one).unwrap();
    assert_eq!(first, randomize_random_values_state(&node, two).unwrap());
    let other = SavedModuleState::RandomValues {
        values: [[1f32.to_bits(); 16]; 7],
        rng: 9988,
    };
    assert_ne!(first, randomize_random_values_state(&node, other).unwrap());
    assert!(node.params.is_empty());
}
