use openpatch_core::*;
use serde_json::json;
fn patch(params: serde_json::Value) -> Patch {
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"e","kind":"adsr_eg","params":params}],"output":"e"})).unwrap()
}
fn sample(engine: &mut Engine) -> PortFrame {
    let mut frame = [PortFrame::default()];
    engine.process_poly_block(&mut frame);
    assert!(frame[0].values.iter().all(|v| v.is_finite()));
    frame[0]
}
fn press(engine: &mut Engine) -> ModuleHandle {
    let h = engine
        .resolve_module_action("e", ModuleAction::PushPress)
        .unwrap();
    assert!(engine.handle_module_event(h, ModuleAction::PushPress));
    h
}
#[test]
fn catalog_parameters_ports_actions_and_version_are_explicit() {
    let catalog = catalog();
    let m = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "adsr_eg")
        .unwrap();
    assert_eq!(
        m["inputs"],
        json!([
            "attack_cv",
            "decay_cv",
            "sustain_cv",
            "release_cv",
            "gate",
            "retrigger"
        ])
    );
    assert_eq!(m["outputs"], json!(["out"]));
    assert_eq!(m["runtime_actions"], json!(["push_press", "push_release"]));
    assert_eq!(
        m["light_names"],
        json!(["attack", "decay", "sustain", "release", "gate"])
    );
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["polyphony"]["primary_input"], "gate");
    for name in ["attack", "decay", "sustain", "release"] {
        assert_eq!(m["params"][name]["default"], 0.5);
        assert_eq!(m["params"][name]["randomization"], "linear");
        let amount = format!("{name}_amount");
        assert_eq!(m["params"][&amount]["default"], 0.0);
        assert_eq!(m["params"][&amount]["randomization"], "disabled");
    }
    for bad in [
        json!({"attack":-0.1}),
        json!({"release":1.1}),
        json!({"sustain_amount":1.1}),
        json!({"decay_amount":-1.1}),
        json!({"push":1.0}),
        json!({"unknown":0.0}),
    ] {
        assert!(patch(bad).compile().is_err());
    }
    let mut p = patch(json!({}));
    p.version = 1;
    assert!(p.compile().is_err());
    let mut p = patch(json!({}));
    p.output_port = "env".into();
    assert!(p.compile().is_err());
    let mut p = patch(json!({}));
    p.nodes[0].params.insert("attack".into(), f32::NAN);
    assert!(p.compile().is_err());
}
#[test]
fn parameter_initialize_and_randomize_preserve_cv_depth_policy() {
    let mut p = patch(json!({"attack":0.2,"attack_amount":-0.75,"decay_amount":0.25}));
    let actions = ModuleParameterActions::new(48_000);
    actions.randomize(&mut p.nodes[0], 1234).unwrap();
    assert_eq!(p.nodes[0].params["attack_amount"], -0.75);
    assert_eq!(p.nodes[0].params["decay_amount"], 0.25);
    assert_ne!(p.nodes[0].params["attack"], 0.2);
    actions.initialize(&mut p.nodes[0]).unwrap();
    for name in ["attack", "decay", "sustain", "release"] {
        assert_eq!(p.nodes[0].params[name], 0.5);
        assert_eq!(p.nodes[0].params[&format!("{name}_amount")], 0.0);
    }
}
#[test]
fn action_validation_bounded_queue_and_final_held_state() {
    let mut engine = patch(json!({})).compile().unwrap();
    let h = engine.resolve_module_control("e").unwrap();
    assert!(engine.resolve_module_lights("e").is_ok());
    assert_eq!(engine.module_saved_state(h), None);
    for bad in [
        ModuleAction::Clock,
        ModuleAction::HoldPress,
        ModuleAction::HoldRelease,
        ModuleAction::BPress,
        ModuleAction::GatePress,
        ModuleAction::PulsePress(0),
    ] {
        assert!(engine.resolve_module_action("e", bad).is_err());
        assert!(!engine.handle_module_event(h, bad));
    }
    assert!(engine.handle_module_event(h, ModuleAction::PushPress));
    assert!(engine.handle_module_event(h, ModuleAction::PushRelease));
    for _ in 0..32 {
        assert_eq!(sample(&mut engine), PortFrame::mono(0.0));
    }
    for _ in 0..32 {
        assert!(engine.handle_module_event(h, ModuleAction::PushRelease));
        assert!(engine.handle_module_event(h, ModuleAction::PushPress));
    }
    assert!(!engine.handle_module_event(h, ModuleAction::PushRelease));
    assert!(sample(&mut engine).values[0] > 0.0);
    assert_eq!(engine.module_control_state(h).unwrap().pressed_mask, 1);
    assert!(engine.module_lights(h).unwrap().manual);
    assert!(engine.release_module_controls(h));
    assert!(!engine.module_control_state(h).unwrap().pressed);
    assert!(sample(&mut engine).values[0] > 0.0);
    let foreign = patch(json!({}))
        .compile()
        .unwrap()
        .resolve_module_state("e")
        .unwrap();
    assert!(!engine.handle_module_event(foreign, ModuleAction::PushPress));
}
#[test]
fn gate_controls_width_and_cables_preserve_single_sample_schedule() {
    let mut p:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"g","kind":"poly_constant","params":{"channels":5,"v1":10,"v2":0,"v3":0,"v4":0,"v5":10}},{"id":"e","kind":"adsr_eg"}],"cables":[{"from":"g","to":"e","input":"gate"}],"output":"e"})).unwrap();
    let mut engine = p.compile().unwrap();
    assert_eq!(sample(&mut engine).channels, 1); // cable starts disconnected
    assert_eq!(sample(&mut engine).channels, 5);
    for _ in 0..30 {
        sample(&mut engine);
    }
    let frame = sample(&mut engine);
    assert!(frame.values[0] > 0.0 && frame.values[4] > 0.0);
    assert_eq!(frame.values[1..4], [0.0; 3]);
    assert_eq!(frame.values[5..], [0.0; 11]);
    p.nodes[0].params.insert("channels".into(), 1.0);
    let mut mono = p.compile().unwrap();
    mono.adopt_state_from(&engine);
    for _ in 0..40 {
        sample(&mut mono);
    }
    assert_eq!(sample(&mut mono).channels, 1);
    let h = mono.resolve_module_lights("e").unwrap();
    for _ in 0..128 {
        sample(&mut mono);
    }
    assert_eq!(mono.module_lights(h).unwrap().input_channels, Some(1));
}
#[test]
fn adoption_retains_history_and_receivers_pending_actions_across_parameter_and_cable_edits() {
    let p = patch(json!({}));
    let mut old = p.compile().unwrap();
    let h = press(&mut old);
    for _ in 0..29 {
        sample(&mut old);
    }
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&old);
    for _ in 0..130 {
        assert_eq!(sample(&mut old), sample(&mut same));
    }
    assert_eq!(
        old.module_lights(h),
        same.module_lights(same.resolve_module_lights("e").unwrap())
    );
    // Old pending release is not inherited. Receiving release is preserved.
    old.handle_module_event(h, ModuleAction::PushRelease);
    let mut untouched = p.compile().unwrap();
    untouched.adopt_state_from(&old);
    sample(&mut untouched);
    assert!(
        untouched
            .module_control_state(untouched.resolve_module_control("e").unwrap())
            .unwrap()
            .pressed
    );
    let mut edited = patch(json!({"attack":0.0})).compile().unwrap();
    let he = edited.resolve_module_control("e").unwrap();
    edited.handle_module_event(he, ModuleAction::PushRelease);
    edited.adopt_state_from(&old);
    let value = sample(&mut edited).values[0];
    assert!(value > 0.0);
    assert!(!edited.module_control_state(he).unwrap().pressed);
    let mut rp = p.clone();
    rp.sample_rate = 96_000;
    let mut fresh = rp.compile().unwrap();
    fresh.adopt_state_from(&old);
    assert_eq!(sample(&mut fresh), PortFrame::mono(0.0));
    assert!(
        !fresh
            .module_control_state(fresh.resolve_module_control("e").unwrap())
            .unwrap()
            .pressed
    );
}
#[test]
fn bypass_freezes_dsp_and_lamps_and_releases_controls_on_both_transitions() {
    let p = patch(json!({}));
    let mut old = p.compile().unwrap();
    let h = press(&mut old);
    for _ in 0..160 {
        sample(&mut old);
    }
    old.handle_module_event(h, ModuleAction::PushPress);
    let lights = old.module_lights(h).unwrap();
    let mut bp = p.clone();
    bp.nodes[0].bypassed = true;
    let mut bypassed = bp.compile().unwrap();
    bypassed.adopt_state_from(&old);
    let hb = bypassed.resolve_module_control("e").unwrap();
    assert!(!bypassed.module_control_state(hb).unwrap().pressed);
    assert!(bypassed
        .resolve_module_action("e", ModuleAction::PushPress)
        .is_err());
    assert!(!bypassed.handle_module_event(hb, ModuleAction::PushPress));
    assert!(bypassed
        .resolve_module_action("e", ModuleAction::PushRelease)
        .is_ok());
    assert!(bypassed.handle_module_event(hb, ModuleAction::PushRelease));
    for _ in 0..300 {
        assert_eq!(sample(&mut bypassed), PortFrame::mono(0.0));
    }
    let frozen = bypassed.module_lights(hb).unwrap();
    assert_eq!(frozen.brightness, lights.brightness);
    assert!(!frozen.manual);
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    old.release_module_controls(h);
    assert_eq!(sample(&mut resumed), sample(&mut old));
    assert!(
        !resumed
            .module_control_state(resumed.resolve_module_control("e").unwrap())
            .unwrap()
            .pressed
    );
}
#[test]
fn initialize_and_reset_clear_envelopes_dividers_lamps_and_held_actions() {
    for initialize in [false, true] {
        let mut e = patch(json!({})).compile().unwrap();
        let h = press(&mut e);
        for _ in 0..129 {
            sample(&mut e);
        }
        assert!(e.module_lights(h).unwrap().brightness[4][0] > 0.0);
        e.handle_module_event(h, ModuleAction::PushPress);
        assert!(if initialize {
            e.initialize_module_state(h)
        } else {
            e.reset_module_state(h)
        });
        assert!(!e.module_control_state(h).unwrap().pressed);
        assert_eq!(
            e.module_lights(h).unwrap().brightness,
            [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
        );
        assert_eq!(sample(&mut e), PortFrame::mono(0.0));
        press(&mut e);
        for _ in 0..14 {
            assert_eq!(sample(&mut e).values[0], 0.0);
        }
        assert!(sample(&mut e).values[0] > 0.0);
    }
}
#[test]
fn block_partitioning_is_bit_identical_for_envelope_and_lamps() {
    let p = patch(json!({"attack":0.01,"decay":0.2,"sustain":0.3,"release":0.01}));
    let mut whole = p.compile().unwrap();
    let hw = press(&mut whole);
    let mut pieces = p.compile().unwrap();
    let hp = press(&mut pieces);
    let mut expected = [PortFrame::default(); 777];
    whole.process_poly_block(&mut expected);
    let mut actual = [PortFrame::default(); 777];
    let mut offset = 0;
    for length in [0, 1, 15, 16, 31, 64, 127, 128, 256, 139] {
        pieces.process_poly_block(&mut actual[offset..offset + length]);
        offset += length;
    }
    assert_eq!(offset, 777);
    assert_eq!(expected, actual);
    assert_eq!(whole.module_lights(hw), pieces.module_lights(hp));
    whole.handle_module_event(hw, ModuleAction::PushRelease);
    pieces.handle_module_event(hp, ModuleAction::PushRelease);
    whole.process_poly_block(&mut expected);
    for chunk in actual.chunks_mut(13) {
        pieces.process_poly_block(chunk);
    }
    assert_eq!(expected, actual);
    assert_eq!(whole.module_lights(hw), pieces.module_lights(hp));
}
