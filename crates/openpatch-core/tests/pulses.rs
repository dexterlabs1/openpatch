use openpatch_core::{
    catalog, validate_module_events, Engine, ModuleAction, Node, Patch, PortFrame,
    TimedModuleEvent, MAX_MODULE_LIGHTS, MODULE_ACTION_NAMES,
};
use serde_json::json;

fn patch(rate: u32, port: &str) -> Patch {
    Patch {
        sample_rate: rate,
        nodes: vec![Node {
            id: "p".into(),
            kind: "pulses".into(),
            bypassed: false,
            params: Default::default(),
        }],
        output: Some("p".into()),
        output_port: port.into(),
        ..Patch::new_v2()
    }
}
fn sample(engine: &mut Engine) -> PortFrame {
    let mut frames = [PortFrame::default()];
    engine.process_poly_block(&mut frames);
    frames[0]
}
#[test]
fn catalog_twenty_mono_ports_no_saved_buttons_and_version_two_only() {
    let c = catalog();
    let m = c["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "pulses")
        .unwrap();
    let expected: Vec<String> = ["trig", "gate"]
        .into_iter()
        .flat_map(|prefix| (1..=10).map(move |row| format!("{prefix}{row}")))
        .collect();
    assert_eq!(m["outputs"], json!(expected));
    assert_eq!(m["inputs"], json!([]));
    assert_eq!(m["params"], json!({}));
    assert_eq!(m["min_patch_version"], 2);
    assert_eq!(m["runtime_actions"], json!(&MODULE_ACTION_NAMES[7..]));
    assert_eq!(m["light_names"].as_array().unwrap().len(), 10);
    assert_eq!(MAX_MODULE_LIGHTS, 10);
    for metadata in m["output_metadata"].as_array().unwrap() {
        assert_eq!(metadata["maximum_channels"], 1);
        assert_eq!(metadata["units"], "V");
    }
    for (index, port) in expected.iter().enumerate() {
        let mut engine = patch(48_000, port).compile().unwrap();
        let action = engine
            .resolve_module_action("p", ModuleAction::PulsePress((index % 10 + 1) as u8))
            .unwrap();
        assert!(
            engine.handle_module_event(action, ModuleAction::PulsePress((index % 10 + 1) as u8))
        );
        assert_eq!(sample(&mut engine), PortFrame::mono(10.0));
    }
    for invalid in ["out", "trig0", "trig11", "trig01", "gate11"] {
        assert!(patch(48_000, invalid).compile().is_err());
    }
    let mut p = patch(48_000, "trig1");
    p.version = 1;
    assert!(p.compile().is_err());
    p.version = 2;
    p.nodes[0].params.insert("button1".into(), 1.0);
    assert!(p.compile().is_err());
}
#[test]
fn all_action_names_round_trip_only_as_strings_and_reject_invalid_channels() {
    let expected = [
        ModuleAction::Clock,
        ModuleAction::Run,
        ModuleAction::Reset,
        ModuleAction::BPress,
        ModuleAction::BRelease,
        ModuleAction::GatePress,
        ModuleAction::GateRelease,
    ];
    for (name, action) in
        MODULE_ACTION_NAMES
            .iter()
            .zip(expected.into_iter().chain((1..=10).flat_map(|row| {
                [
                    ModuleAction::PulsePress(row),
                    ModuleAction::PulseRelease(row),
                ]
            })))
    {
        assert_eq!(serde_json::to_value(action).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<ModuleAction>(json!(name)).unwrap(),
            action
        );
    }
    for row in 1..=10 {
        assert_eq!(
            ModuleAction::PulsePress(row).pulse_button(),
            Some((usize::from(row - 1), true))
        );
        assert_eq!(
            ModuleAction::PulseRelease(row).pulse_button(),
            Some((usize::from(row - 1), false))
        );
    }
    for value in [
        json!({"pulse1_press":null}),
        json!({"PulsePress":1}),
        json!(1),
        json!(null),
        json!("pulse0_press"),
        json!("pulse11_release"),
        json!("pulse01_press"),
        json!("pulse1_press "),
    ] {
        assert!(serde_json::from_value::<ModuleAction>(value).is_err());
    }
    for invalid in [
        ModuleAction::PulsePress(0),
        ModuleAction::PulsePress(11),
        ModuleAction::PulseRelease(0),
        ModuleAction::PulseRelease(255),
    ] {
        assert_eq!(invalid.pulse_button(), None);
        assert!(serde_json::to_value(invalid).is_err());
        assert!(validate_module_events(
            &[TimedModuleEvent {
                frame: 0,
                id: "p".into(),
                action: invalid
            }],
            1
        )
        .is_err());
    }
    let events: Vec<_> = (0..64)
        .map(|index| TimedModuleEvent {
            frame: 0,
            id: "p".into(),
            action: if index % 2 == 0 {
                ModuleAction::PulsePress(10)
            } else {
                ModuleAction::PulseRelease(10)
            },
        })
        .collect();
    assert!(validate_module_events(&events, 1).is_ok());
    let mut excessive = events;
    excessive.push(TimedModuleEvent {
        frame: 0,
        id: "p".into(),
        action: ModuleAction::PulsePress(1),
    });
    assert!(validate_module_events(&excessive, 1).is_err());
}
#[test]
fn invalid_actions_and_foreign_handles_do_not_change_any_button() {
    let mut e = patch(48_000, "gate10").compile().unwrap();
    let h = e.resolve_module_control("p").unwrap();
    for action in [
        ModuleAction::PulsePress(0),
        ModuleAction::PulseRelease(11),
        ModuleAction::Clock,
        ModuleAction::Run,
        ModuleAction::BPress,
        ModuleAction::GateRelease,
    ] {
        assert!(e.resolve_module_action("p", action).is_err());
        assert!(!e.handle_module_event(h, action));
    }
    let other = patch(48_000, "gate10").compile().unwrap();
    assert!(!e.handle_module_event(
        other.resolve_module_control("p").unwrap(),
        ModuleAction::PulsePress(10)
    ));
    assert_eq!(sample(&mut e), PortFrame::mono(0.0));
    assert_eq!(e.module_control_state(h).unwrap().pressed_mask, 0);
    for kind in ["seq3", "logic", "process"] {
        let mut p = patch(
            48_000,
            if kind == "seq3" {
                "cv1"
            } else if kind == "logic" {
                "or"
            } else {
                "slew"
            },
        );
        p.nodes[0].kind = kind.into();
        let mut other = p.compile().unwrap();
        let h = other
            .resolve_module_control("p")
            .or_else(|_| other.resolve_module_event("p"))
            .unwrap();
        assert!(other
            .resolve_module_action("p", ModuleAction::PulsePress(1))
            .is_err());
        assert!(!other.handle_module_event(h, ModuleAction::PulsePress(1)));
    }
}
#[test]
fn compatible_adoption_bypass_freeze_release_rate_reset_and_initialization() {
    let p = patch(48_000, "trig10");
    let mut active = p.compile().unwrap();
    let h = active.resolve_module_control("p").unwrap();
    assert!(active.handle_module_event(h, ModuleAction::PulsePress(10)));
    for _ in 0..7 {
        assert_eq!(sample(&mut active), PortFrame::mono(10.0));
    }
    let mut compatible = p.compile().unwrap();
    compatible.adopt_state_from(&active);
    let hc = compatible.resolve_module_control("p").unwrap();
    assert_eq!(
        compatible.module_control_state(hc).unwrap().pressed_mask,
        1 << 9
    );
    assert_eq!(
        compatible.module_lights(hc).unwrap().brightness[9],
        [1.0, 0.0]
    );
    assert_eq!(sample(&mut compatible), PortFrame::mono(10.0));
    let mut bypass_patch = p.clone();
    bypass_patch.nodes[0].bypassed = true;
    let mut bypassed = bypass_patch.compile().unwrap();
    bypassed.adopt_state_from(&compatible);
    let hb = bypassed.resolve_module_control("p").unwrap();
    assert_eq!(bypassed.module_control_state(hb).unwrap().pressed_mask, 0);
    assert!(bypassed
        .resolve_module_action("p", ModuleAction::PulsePress(10))
        .is_err());
    assert!(!bypassed.handle_module_event(hb, ModuleAction::PulsePress(10)));
    for invalid in [
        ModuleAction::PulseRelease(0),
        ModuleAction::PulseRelease(11),
    ] {
        assert!(bypassed.resolve_module_action("p", invalid).is_err());
        assert!(!bypassed.handle_module_event(hb, invalid));
    }
    assert!(bypassed
        .resolve_module_action("p", ModuleAction::PulseRelease(10))
        .is_ok());
    assert!(bypassed.handle_module_event(hb, ModuleAction::PulseRelease(10)));
    for _ in 0..100 {
        assert_eq!(sample(&mut bypassed), PortFrame::mono(0.0));
    }
    // Every output, including output index 19 beyond the input bypass map, is safely silent.
    for port in ["trig1", "trig10", "gate1", "gate10"] {
        bypass_patch.output_port = port.into();
        assert_eq!(
            sample(&mut bypass_patch.compile().unwrap()),
            PortFrame::mono(0.0)
        );
    }
    let mut resumed = p.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    let hr = resumed.resolve_module_control("p").unwrap();
    assert_eq!(resumed.module_control_state(hr).unwrap().pressed_mask, 0);
    for _ in 8..48 {
        assert_eq!(sample(&mut resumed), PortFrame::mono(10.0));
    }
    assert_eq!(sample(&mut resumed), PortFrame::mono(0.0));
    let mut rate_patch = p.clone();
    rate_patch.sample_rate = 96_000;
    let mut rate_changed = rate_patch.compile().unwrap();
    rate_changed.adopt_state_from(&active);
    assert_eq!(sample(&mut rate_changed), PortFrame::mono(0.0));
    assert_eq!(
        rate_changed
            .module_control_state(rate_changed.resolve_module_control("p").unwrap())
            .unwrap()
            .pressed_mask,
        0
    );
    assert!(resumed.handle_module_event(hr, ModuleAction::PulsePress(10)));
    assert_eq!(sample(&mut resumed), PortFrame::mono(10.0));
    assert!(resumed.initialize_module_state(hr));
    assert_eq!(resumed.module_control_state(hr).unwrap().pressed_mask, 0);
    assert_eq!(sample(&mut resumed), PortFrame::mono(0.0));
}
