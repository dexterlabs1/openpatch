use super::*;
fn xfade() -> HostedModule {
    HostedModule::xfade(
        HostedIdentity {
            plugin: "Bogaudio".into(),
            model: "Bogaudio-XFade".into(),
            version: "2.6.47".into(),
            rack_version: "2.6.6".into(),
            plugin_sha256: "a".repeat(64),
            runtime_sha256: "b".repeat(64),
        },
        0.,
        0.5,
        false,
    )
    .unwrap()
}
#[test]
fn genuine_xfade_snapshot_preserves_all_parameters_and_empty_custom_state() {
    let module = xfade();
    let genuine: Value = serde_json::from_str(include_str!(
        "../../../../../scripts/rack_binary_host/fixtures/xfade-default-state.json"
    ))
    .unwrap();
    assert_eq!(module.state.as_ref(), &genuine);
    let restored: HostedModule =
        serde_json::from_str(&serde_json::to_string(&module).unwrap()).unwrap();
    restored.validate_supported().unwrap();
    assert_eq!(restored.state, module.state);
    assert!(restored.validate_supported_offset().is_err());
    assert_eq!(
        restored.capability().unwrap().input_keys(),
        ["mix_cv", "a", "b"]
    );
}
#[test]
fn xfade_parameter_ranges_discrete_mode_and_previous_snapshots_are_preserved() {
    let module = xfade();
    for (key, value) in [
        ("mix", -1.),
        ("mix", 1.),
        ("curve", 0.),
        ("curve", 1.),
        ("linear", 1.),
    ] {
        assert_eq!(
            module
                .with_parameter(key, value)
                .unwrap()
                .parameter(key)
                .unwrap(),
            value
        );
    }
    for (key, value) in [
        ("mix", 1.01),
        ("curve", -0.1),
        ("linear", 0.5),
        ("linear", 2.),
        ("curve", f32::NAN),
        ("offset", 0.),
    ] {
        assert!(module.with_parameter(key, value).is_err());
    }
    assert_eq!(module.parameter("mix").unwrap(), 0.);
    assert_eq!(module.parameter("linear").unwrap(), 0.);
    assert!(module.with_option("offset_first", true).is_err());
}
#[test]
fn unsupported_xfade_custom_state_is_preserved_but_never_executed() {
    let mut module = xfade();
    let mut state = module.state.as_ref().clone();
    state["data"]["skin"] = json!("dark");
    module.state = HostedState::new(state).unwrap();
    module.validate_structure().unwrap();
    assert!(module.validate_supported().is_err());
    let roundtrip: HostedModule =
        serde_json::from_value(serde_json::to_value(&module).unwrap()).unwrap();
    assert_eq!(module.state, roundtrip.state);
}
#[test]
fn duplicate_missing_foreign_and_fractional_xfade_snapshot_fields_fail_closed() {
    for fault in [
        "duplicate",
        "missing",
        "foreign",
        "fractional",
        "range",
        "extra",
    ] {
        let mut module = xfade();
        let mut state = module.state.as_ref().clone();
        match fault {
            "duplicate" => state["params"][2]["id"] = json!(1),
            "missing" => {
                state["params"].as_array_mut().unwrap().pop();
            }
            "foreign" => state["model"] = json!("Bogaudio-Offset"),
            "fractional" => state["params"][2]["value"] = json!(0.5),
            "range" => state["params"][1]["value"] = json!(-0.25),
            _ => state["extra"] = json!(0),
        }
        module.state = HostedState::new(state).unwrap();
        assert!(module.validate_supported().is_err(), "{fault}");
    }
    assert!(!HostedCapability::XFade.accepts_input_mask(4));
    assert!(HostedCapability::XFade.accepts_input_mask(2));
    assert!(HostedCapability::XFade.accepts_input_mask(7));
}
