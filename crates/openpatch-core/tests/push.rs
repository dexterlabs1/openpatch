use openpatch_core::*;
use serde_json::json;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        content: None,
        rackmcp_sequence: None,
        score: None,
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ((*name).into(), *value))
            .collect(),
    }
}
fn patch(holding: Option<f32>, port: &str) -> Patch {
    Patch {
        nodes: vec![node(
            "p",
            "push",
            &holding.map(|v| vec![("holding", v)]).unwrap_or_default(),
        )],
        output: Some("p".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn sample(e: &mut Engine) -> f32 {
    let mut out = [PortFrame::default()];
    e.process_poly_block(&mut out);
    assert_eq!(out[0].channels, 1);
    out[0].values[0]
}
#[test]
fn catalog_actions_ports_and_saved_parameter_policy() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "push")
        .unwrap();
    assert_eq!(m["inputs"], json!(["push", "hold"]));
    assert_eq!(m["outputs"], json!(["trig", "gate"]));
    assert_eq!(
        m["runtime_actions"],
        json!(["push_press", "push_release", "hold_press", "hold_release"])
    );
    assert_eq!(m["light_names"], json!(["gate", "hold"]));
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"]["holding"]["default"], 0.0);
    assert_eq!(m["params"]["holding"]["randomization"], "disabled");
    assert_eq!(m["params"]["holding"]["initialize"], true);
    for meta in m["output_metadata"].as_array().unwrap() {
        assert_eq!(meta["maximum_channels"], 1);
    }
    for invalid in [-1.0, 0.5, 2.0, f32::NAN] {
        assert!(patch(Some(invalid), "gate").compile().is_err());
    }
    let mut p = patch(None, "gate");
    p.nodes[0].params.insert("push".into(), 1.0);
    assert!(p.compile().is_err());
    let mut p = patch(None, "gate");
    p.version = 1;
    assert!(p.compile().is_err());
    assert!(patch(None, "out").compile().is_err());
    let actions = ModuleParameterActions::new(48_000);
    let mut n = node("p", "push", &[("holding", 1.0)]);
    actions.randomize(&mut n, 19).unwrap();
    assert_eq!(n.params["holding"], 1.0);
    actions.initialize(&mut n).unwrap();
    assert_eq!(n.params["holding"], 0.0);
}
#[test]
fn strict_action_wire_names_and_kind_matching() {
    for (index, action) in [
        ModuleAction::PushPress,
        ModuleAction::PushRelease,
        ModuleAction::HoldPress,
        ModuleAction::HoldRelease,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            json!(MODULE_ACTION_NAMES[27 + index])
        );
        assert_eq!(
            serde_json::from_value::<ModuleAction>(json!(MODULE_ACTION_NAMES[27 + index])).unwrap(),
            action
        );
        assert!(action.pulse_button().is_none());
        assert!(patch(None, "gate")
            .compile()
            .unwrap()
            .resolve_module_action("p", action)
            .is_ok());
    }
    for invalid in [
        json!({"push_press":null}),
        json!({"HoldPress":null}),
        json!("push1_press"),
        json!("hold"),
        json!(false),
    ] {
        assert!(serde_json::from_value::<ModuleAction>(invalid).is_err());
    }
    let mut e = patch(None, "gate").compile().unwrap();
    let h = e.resolve_module_control("p").unwrap();
    for invalid in [
        ModuleAction::BPress,
        ModuleAction::GateRelease,
        ModuleAction::Run,
        ModuleAction::PulsePress(1),
        ModuleAction::PulseRelease(0),
    ] {
        assert!(e.resolve_module_action("p", invalid).is_err());
        assert!(!e.handle_module_event(h, invalid));
    }
    for (kind, port) in [
        ("seq3", "cv1"),
        ("pulses", "trig1"),
        ("logic", "or"),
        ("process", "slew"),
    ] {
        let mut p = patch(None, port);
        p.nodes[0] = node("p", kind, &[]);
        let mut e = p.compile().unwrap();
        let h = e.resolve_module_state("p").unwrap();
        assert!(e
            .resolve_module_action("p", ModuleAction::HoldPress)
            .is_err());
        assert!(!e.handle_module_event(h, ModuleAction::HoldPress));
    }
}
#[test]
fn generic_saved_state_is_kind_checked_and_existing_seq_run_api_still_works() {
    let push = node("p", "push", &[]);
    let seq = node("s", "seq3", &[]);
    assert_eq!(
        SavedModuleState::from_node(&push),
        Some(SavedModuleState::PushHold(false))
    );
    assert_eq!(
        SavedModuleState::from_node(&seq),
        Some(SavedModuleState::Seq3Run(true))
    );
    assert_eq!(
        SavedModuleState::from_node(&node("c", "constant", &[])),
        None
    );
    for state in [
        SavedModuleState::PushHold(true),
        SavedModuleState::Seq3Run(false),
    ] {
        assert_eq!(state.with_value(!state.value()).value(), !state.value());
        assert_eq!(state.with_value(false).kind(), state.kind());
    }
    assert_eq!(SavedModuleState::PushHold(false).parameter(), "holding");
    assert_eq!(SavedModuleState::Seq3Run(true).parameter(), "running");
    let mut p = patch(None, "gate");
    p.nodes.push(seq);
    let mut e = p.compile().unwrap();
    let hp = e.resolve_module_state("p").unwrap();
    let hs = e.resolve_module_state("s").unwrap();
    assert_eq!(
        e.module_saved_state(hp),
        Some(SavedModuleState::PushHold(false))
    );
    assert_eq!(
        e.module_saved_state(hs),
        Some(SavedModuleState::Seq3Run(true))
    );
    assert!(!e.set_module_saved_state(hp, SavedModuleState::Seq3Run(false)));
    assert!(!e.set_module_saved_state(hs, SavedModuleState::PushHold(true)));
    assert!(e.set_module_saved_state(hp, SavedModuleState::PushHold(true)));
    assert_eq!(sample(&mut e), 10.0);
    assert!(e.set_module_saved_state(hs, SavedModuleState::Seq3Run(false)));
    assert!(!e.module_telemetry(hs).unwrap().running);
    assert!(e.set_module_running(hs, true));
    assert_eq!(
        e.module_saved_state(hs),
        Some(SavedModuleState::Seq3Run(true))
    );
    assert!(!e.set_module_running(hp, false));
    let foreign = p.compile().unwrap().resolve_module_state("p").unwrap();
    assert_eq!(e.module_saved_state(foreign), None);
    assert!(!e.set_module_saved_state(foreign, SavedModuleState::PushHold(false)));
    assert!(e.handle_module_event(hp, ModuleAction::PushPress));
    sample(&mut e);
    // Saved-state assignment doesn't release physical controls or replay actions.
    assert!(e.set_module_saved_state(hp, SavedModuleState::PushHold(false)));
    assert_eq!(e.module_control_state(hp).unwrap().pressed_mask, 1);
    assert_eq!(sample(&mut e), 10.0);
}
#[test]
fn compatible_adoption_saved_intent_override_bypass_and_sample_rate_freshness() {
    let p = patch(None, "trig");
    let mut old = p.compile().unwrap();
    let h = old.resolve_module_control("p").unwrap();
    old.handle_module_event(h, ModuleAction::HoldPress);
    assert_eq!(sample(&mut old), 10.0);
    let mut same = p.compile().unwrap();
    same.adopt_state_from(&old);
    let hs = same.resolve_module_control("p").unwrap();
    assert_eq!(
        same.module_saved_state(hs),
        Some(SavedModuleState::PushHold(true))
    );
    assert_eq!(same.module_control_state(hs).unwrap().pressed_mask, 2);
    let mut explicit = patch(Some(0.0), "trig").compile().unwrap();
    explicit.adopt_state_from(&old);
    let he = explicit.resolve_module_state("p").unwrap();
    assert_eq!(
        explicit.module_saved_state(he),
        Some(SavedModuleState::PushHold(false))
    );
    assert_eq!(sample(&mut explicit), 10.0); // tail retained despite latch override
    let mut bp = p.clone();
    bp.nodes[0].bypassed = true;
    let mut bypassed = bp.compile().unwrap();
    bypassed.adopt_state_from(&old);
    let hb = bypassed.resolve_module_control("p").unwrap();
    assert_eq!(bypassed.module_control_state(hb).unwrap().pressed_mask, 0);
    assert_eq!(
        bypassed.module_saved_state(hb),
        Some(SavedModuleState::PushHold(true))
    );
    for action in [ModuleAction::PushPress, ModuleAction::HoldPress] {
        assert!(bypassed.resolve_module_action("p", action).is_err());
        assert!(!bypassed.handle_module_event(hb, action));
    }
    for action in [ModuleAction::PushRelease, ModuleAction::HoldRelease] {
        assert!(bypassed.resolve_module_action("p", action).is_ok());
        assert!(bypassed.handle_module_event(hb, action));
    }
    for _ in 0..100 {
        assert_eq!(sample(&mut bypassed), 0.0);
    }
    assert!(bypassed.set_module_saved_state(hb, SavedModuleState::PushHold(true)));
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    let hr = resumed.resolve_module_state("p").unwrap();
    assert_eq!(
        resumed.module_saved_state(hr),
        Some(SavedModuleState::PushHold(true))
    );
    for _ in 1..48 {
        assert_eq!(sample(&mut resumed), 10.0);
    }
    assert_eq!(sample(&mut resumed), 0.0);
    let mut rp = p.clone();
    rp.sample_rate = 96_000;
    let mut fresh = rp.compile().unwrap();
    fresh.adopt_state_from(&old);
    let hf = fresh.resolve_module_state("p").unwrap();
    assert_eq!(
        fresh.module_saved_state(hf),
        Some(SavedModuleState::PushHold(false))
    );
    assert_eq!(sample(&mut fresh), 0.0);
    assert!(resumed.initialize_module_state(hr));
    assert_eq!(
        resumed.module_saved_state(hr),
        Some(SavedModuleState::PushHold(false))
    );
    assert_eq!(resumed.module_control_state(hr).unwrap().pressed_mask, 0);
}
