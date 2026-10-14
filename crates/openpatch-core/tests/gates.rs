use openpatch_core::*;
use serde_json::json;
fn node(id: &str, kind: &str, params: &[(&str, f32)]) -> Node {
    Node {
        score: None,
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: params.iter().map(|(n, v)| ((*n).into(), *v)).collect(),
    }
}
fn cable(from: &str, output: &str, to: &str, input: &str) -> Cable {
    Cable {
        input_order: None,
        output_order: None,
        color: None,
        id: None,
        from: from.into(),
        output: output.into(),
        to: to.into(),
        input: input.into(),
    }
}
fn patch(port: &str) -> Patch {
    Patch {
        nodes: vec![node("g", "gates", &[])],
        output: Some("g".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn sample(e: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    e.process_poly_block(&mut frames);
    frames[0]
}
#[test]
fn catalog_ports_saved_parameter_and_strict_reset_action_contract() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "gates")
        .unwrap();
    assert_eq!(m["inputs"], json!(["length", "in", "reset"]));
    assert_eq!(
        m["outputs"],
        json!(["rise", "fall", "flip", "flop", "gate", "delay"])
    );
    assert_eq!(m["light_names"], m["outputs"]);
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["params"]["length"]["default"], 0.1);
    assert_eq!(m["params"]["length"]["randomization"], "log2");
    assert_eq!(
        m["runtime_actions"],
        json!(["reset_press", "reset_release"])
    );
    assert_eq!(m["delay_history"]["capacity_per_voice"], 1023);
    assert_eq!(m["delay_history"]["allocated_transition_bytes"], 261_888);
    for port in ["rise", "fall", "flip", "flop", "gate", "delay"] {
        assert!(patch(port).compile().is_ok());
    }
    for port in ["out", "rise1", "reset"] {
        assert!(patch(port).compile().is_err());
    }
    let mut p = patch("flip");
    p.version = 1;
    assert!(p.compile().is_err());
    p.version = 2;
    for value in [0.0, 0.0009, 10.1, f32::NAN] {
        p.nodes[0].params.insert("length".into(), value);
        assert!(p.compile().is_err());
    }
    p.nodes[0].params.clear();
    p.nodes[0].params.insert("holding".into(), 1.0);
    assert!(p.compile().is_err());
    for (name, action) in [
        ("reset_press", ModuleAction::ResetPress),
        ("reset_release", ModuleAction::ResetRelease),
    ] {
        assert_eq!(serde_json::to_value(action).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<ModuleAction>(json!(name)).unwrap(),
            action
        );
    }
    for value in [
        json!({"reset_press":null}),
        json!("ResetPress"),
        json!("reset1_press"),
        json!(false),
    ] {
        assert!(serde_json::from_value::<ModuleAction>(value).is_err());
    }
    let mut e = patch("flip").compile().unwrap();
    let h = e.resolve_module_control("g").unwrap();
    for action in [
        ModuleAction::Reset,
        ModuleAction::Clock,
        ModuleAction::PushPress,
        ModuleAction::GateRelease,
        ModuleAction::PulsePress(1),
    ] {
        assert!(e.resolve_module_action("g", action).is_err());
        assert!(!e.handle_module_event(h, action));
    }
    assert_eq!(e.module_saved_state(h), None);
    assert_eq!(SavedModuleState::from_node(&p.nodes[0]), None);
    for (kind, port) in [
        ("seq3", "cv1"),
        ("push", "gate"),
        ("pulses", "trig1"),
        ("logic", "or"),
    ] {
        let mut p = patch(port);
        p.nodes[0] = node("g", kind, &[]);
        let mut e = p.compile().unwrap();
        let h = e.resolve_module_state("g").unwrap();
        assert!(e
            .resolve_module_action("g", ModuleAction::ResetPress)
            .is_err());
        assert!(!e.handle_module_event(h, ModuleAction::ResetPress));
    }
}
#[test]
fn actual_engine_poly_ports_and_channel_local_reset() {
    let mut p = patch("flop");
    p.nodes
        .insert(0, node("v", "poly_constant", &[("channels", 16.0)]));
    p.nodes.push(node("reset", "constant", &[("value", 0.0)]));
    p.cables = vec![
        cable("v", "out", "g", "in"),
        cable("reset", "out", "g", "reset"),
    ];
    let mut old = p.compile().unwrap();
    sample(&mut old);
    let out = sample(&mut old);
    assert_eq!(out.channels, 16);
    assert_eq!(out.values[0], 0.0);
    assert!(out.values[1..].iter().all(|v| *v == 10.0));
    p.nodes
        .iter_mut()
        .find(|n| n.id == "reset")
        .unwrap()
        .params
        .insert("value".into(), 2.0);
    let mut updated = p.compile().unwrap();
    updated.adopt_state_from(&old);
    sample(&mut updated);
    let out = sample(&mut updated);
    assert_eq!(out.values[1], 10.0);
    assert!(out.values[2..].iter().all(|v| *v == 10.0));
    // Lane one was already low; mono reset cannot alter any higher lane.
    assert_eq!(out.values[0], 0.0);
    let h = updated.resolve_module_control("g").unwrap();
    assert!(updated.handle_module_event(h, ModuleAction::ResetPress));
    assert!(updated.handle_module_event(h, ModuleAction::ResetRelease));
    let out = sample(&mut updated);
    assert!(out.values.iter().all(|v| *v == 0.0));
    assert_eq!(updated.module_control_state(h).unwrap().pressed_mask, 0);
}
#[test]
fn adopted_history_can_be_revealed_after_unconnected_delay_and_new_length() {
    let mut p = patch("flop");
    p.nodes
        .insert(0, node("source", "constant", &[("value", 2.0)]));
    p.nodes[1].params.insert("length".into(), 0.001);
    p.cables.push(cable("source", "out", "g", "in"));
    let mut old = p.compile().unwrap();
    let mut frames = [PortFrame::default(); 32];
    old.process_poly_block(&mut frames);
    // DELAY had no sink/cable; switching the selected output must expose history.
    p.output_port = "delay".into();
    let mut selected = p.compile().unwrap();
    selected.adopt_state_from(&old);
    let mut reference = p.compile().unwrap();
    reference.process_poly_block(&mut frames);
    let mut actual = [PortFrame::default(); 128];
    let mut expected = actual;
    selected.process_poly_block(&mut actual);
    reference.process_poly_block(&mut expected);
    assert_eq!(actual, expected);
    assert_eq!(actual[18].values[0], 10.0);
    // A longer duration can query earlier retained events without consuming them.
    p.nodes[1].params.insert("length".into(), 0.002);
    let mut longer = p.compile().unwrap();
    longer.adopt_state_from(&selected);
    assert_eq!(sample(&mut longer).values[0], 10.0);
}
#[test]
fn compatible_adoption_initialize_bypass_release_and_fresh_rate_policy() {
    let mut p = patch("flop");
    p.nodes
        .insert(0, node("source", "constant", &[("value", 2.0)]));
    p.cables.push(cable("source", "out", "g", "in"));
    let mut old = p.compile().unwrap();
    sample(&mut old);
    assert_eq!(sample(&mut old).values[0], 10.0);
    let h = old.resolve_module_control("g").unwrap();
    assert!(old.handle_module_event(h, ModuleAction::ResetRelease));
    assert!(old.initialize_module_state(h));
    assert!(old.release_module_controls(h));
    assert_eq!(sample(&mut old).values[0], 10.0);
    let mut replacement = p.compile().unwrap();
    replacement.adopt_state_from(&old);
    let hr = replacement.resolve_module_control("g").unwrap();
    assert_eq!(sample(&mut replacement).values[0], 10.0);
    let mut bp = p.clone();
    bp.nodes.iter_mut().find(|n| n.id == "g").unwrap().bypassed = true;
    let mut bypassed = bp.compile().unwrap();
    bypassed.adopt_state_from(&replacement);
    let hb = bypassed.resolve_module_control("g").unwrap();
    assert!(bypassed
        .resolve_module_action("g", ModuleAction::ResetPress)
        .is_err());
    assert!(!bypassed.handle_module_event(hb, ModuleAction::ResetPress));
    assert!(bypassed
        .resolve_module_action("g", ModuleAction::ResetRelease)
        .is_ok());
    assert!(bypassed.handle_module_event(hb, ModuleAction::ResetRelease));
    for _ in 0..100 {
        assert_eq!(sample(&mut bypassed), PortFrame::mono(0.0));
    }
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    assert_eq!(sample(&mut resumed).values[0], 10.0);
    // A fresh sample rate starts LOW; its first empty edge frame is still idle.
    p.sample_rate = 96_000;
    let mut fresh = p.compile().unwrap();
    fresh.adopt_state_from(&old);
    assert_eq!(sample(&mut fresh).values[0], 0.0);
    let foreign = fresh.resolve_module_control("g").unwrap();
    assert!(!replacement.handle_module_event(foreign, ModuleAction::ResetPress));
    assert!(replacement.handle_module_event(hr, ModuleAction::ResetPress));
    assert!(replacement.handle_module_event(hr, ModuleAction::ResetRelease));
    assert_eq!(sample(&mut replacement).values[0], 0.0);
    assert!(replacement.reset_module_state(hr));
    assert_eq!(
        replacement.module_control_state(hr).unwrap().pressed_mask,
        0
    );
}
