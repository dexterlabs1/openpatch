use openpatch_core::*;
use serde_json::json;
fn patch() -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"s","kind":"sh_asr"}],"output":"s","output_port":"out1"})).unwrap()
}
fn sample(e: &mut Engine) -> PortFrame {
    let mut f = [PortFrame::default()];
    e.process_poly_block(&mut f);
    f[0]
}
fn state(e: &Engine) -> SavedModuleState {
    e.module_saved_state(e.resolve_module_state("s").unwrap())
        .unwrap()
}
fn click(e: &mut Engine) {
    let h = e.resolve_module_control("s").unwrap();
    assert!(e.handle_module_event(h, ModuleAction::PushPress));
    assert!(e.handle_module_event(h, ModuleAction::PushRelease));
}
#[test]
fn catalog_version_ports_validation_and_whole_module_randomize_policy() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "sh_asr")
        .unwrap();
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["inputs"].as_array().unwrap().len(), 16);
    assert_eq!(
        m["outputs"],
        json!(["out1", "out2", "out3", "out4", "out5", "out6", "out7", "out8"])
    );
    assert_eq!(m["params"].as_object().unwrap().len(), 13);
    assert_eq!(
        m["runtime_actions"],
        json!(["push_press", "push_release", "reset_press", "reset_release"])
    );
    for p in m["params"].as_object().unwrap().values() {
        assert_eq!(p["randomization"], "disabled");
    }
    for out in m["output_metadata"].as_array().unwrap() {
        assert_eq!(out["maximum_channels"], 1);
    }
    let mut p = patch();
    p.version = 1;
    assert!(p.compile().is_err());
    for (name, v) in [
        ("random", 0.5),
        ("random", 2.),
        ("range", -1.),
        ("range", 6.),
        ("range", 1.5),
        ("seed", 0.5),
        ("seed", 16777216.),
        ("value01", 0.),
        ("value9", 0.),
        ("value1", 0.),
        ("rng_lo", 1.),
    ] {
        let mut p = patch();
        p.nodes[0].params.insert(name.into(), v);
        assert!(p.compile().is_err(), "{name}");
    }
    for port in ["out", "out0", "out9", "out01"] {
        let mut p = patch();
        p.output_port = port.into();
        assert!(p.compile().is_err());
    }
    let mut n = patch().nodes.remove(0);
    let actions = ModuleParameterActions::new(48000);
    let initial = initial_shift_register_state(&n).unwrap();
    initial.write_params(&mut n.params);
    let before = n.params.clone();
    actions.randomize(&mut n, 333).unwrap();
    assert_eq!(n.params, before);
    actions.initialize(&mut n).unwrap();
    assert_eq!(initial_shift_register_state(&n).unwrap(), initial);
}
#[test]
fn all_sixteen_inputs_and_eight_mono_outputs_route_and_zero_channel_normaling() {
    for row in 1..=8 {
        let mut p = patch();
        p.output_port = format!("out{row}");
        p.nodes.push(serde_json::from_value(json!({"id":"input","kind":"poly_constant","params":{"channels":16,"v1":12,"v2":9}})).unwrap());
        p.cables.push(
            serde_json::from_value(json!({"from":"input","to":"s","input":format!("in{row}")}))
                .unwrap(),
        );
        // A patched clock breaks the manual chain; initial high does not clock it.
        p.nodes.push(
            serde_json::from_value(
                json!({"id":"clock","kind":"poly_constant","params":{"channels":0}}),
            )
            .unwrap(),
        );
        p.cables.push(
            serde_json::from_value(json!({"from":"clock","to":"s","input":format!("trig{row}")}))
                .unwrap(),
        );
        let mut e = p.compile().unwrap();
        sample(&mut e);
        sample(&mut e);
        click(&mut e);
        assert_eq!(sample(&mut e), PortFrame::mono(12.));
        p.nodes[2].params.insert("channels".into(), 1.);
        let mut patched = p.compile().unwrap();
        sample(&mut patched);
        sample(&mut patched);
        click(&mut patched);
        assert_eq!(
            sample(&mut patched),
            PortFrame::mono(if row == 1 { 12. } else { 0. })
        );
    }
}
#[test]
fn persistence_exact_resume_restore_override_adoption_and_initialize() {
    let mut p = patch();
    p.nodes[0].params.insert("random".into(), 1.);
    let mut live = p.compile().unwrap();
    click(&mut live);
    sample(&mut live);
    let held = state(&live);
    held.write_params(&mut p.nodes[0].params);
    let encoded = serde_json::to_vec(&p).unwrap();
    let p: Patch = serde_json::from_slice(&encoded).unwrap();
    let mut loaded = p.compile().unwrap();
    assert_eq!(state(&loaded), held);
    assert_eq!(SavedModuleState::from_node(&p.nodes[0]), Some(held));
    for e in [&mut live, &mut loaded] {
        click(e);
        sample(e);
    }
    assert_eq!(state(&live), state(&loaded));
    let mut adopted = p.compile().unwrap();
    adopted.adopt_state_from(&loaded);
    assert_eq!(state(&adopted), state(&live));
    let h = adopted.resolve_module_state("s").unwrap();
    assert!(adopted.set_module_saved_state(h, held));
    assert_eq!(state(&adopted), held);
    assert!(!adopted.set_module_saved_state(h, SavedModuleState::PushHold(false)));
    assert!(adopted.initialize_module_state(h));
    assert_eq!(
        state(&adopted),
        SavedModuleState::ShiftRegister {
            values: [0; 8],
            rng: 1 ^ 0xa341316c
        }
    );
    let mut other = p.clone();
    other.sample_rate = 96000;
    let mut fresh = other.compile().unwrap();
    fresh.adopt_state_from(&live);
    assert_eq!(state(&fresh), held);
}
#[test]
fn bypass_freezes_rng_holds_and_releases_controls_and_pending_actions() {
    let mut p = patch();
    p.nodes[0].params.insert("random".into(), 1.);
    let mut live = p.compile().unwrap();
    let h = live.resolve_module_control("s").unwrap();
    live.handle_module_event(h, ModuleAction::PushPress);
    sample(&mut live);
    let held = state(&live);
    live.handle_module_event(h, ModuleAction::ResetPress);
    p.nodes[0].bypassed = true;
    let mut paused = p.compile().unwrap();
    paused.adopt_state_from(&live);
    let h = paused.resolve_module_control("s").unwrap();
    assert!(!paused.module_control_state(h).unwrap().pressed);
    assert!(!paused.handle_module_event(h, ModuleAction::ResetPress));
    assert!(paused.handle_module_event(h, ModuleAction::ResetRelease));
    assert!(paused.handle_module_event(h, ModuleAction::PushRelease));
    for _ in 0..20 {
        assert_eq!(sample(&mut paused), PortFrame::mono(0.));
    }
    assert_eq!(state(&paused), held);
    p.nodes[0].bypassed = false;
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&paused);
    assert_eq!(
        sample(&mut resumed).values[0],
        match held {
            SavedModuleState::ShiftRegister { values, .. } => f32::from_bits(values[0]),
            _ => unreachable!(),
        }
    );
    assert_eq!(state(&resumed), held);
}
