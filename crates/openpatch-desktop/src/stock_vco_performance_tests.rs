#[test]
fn stock_vco_is_passive_and_five_lamps_obey_packet_identity_without_clock_metadata() {
    assert!(ModuleKind::from_name("stock_vco") == Some(ModuleKind::StockVco));
    assert_eq!(ModuleKind::StockVco.name(), "stock_vco");
    assert!(!ModuleKind::StockVco.manual() && !ModuleKind::StockVco.sequential());
    assert!(!ModuleKind::StockVco.accepts(ModuleAction::BPress));
    let entry = Entry {
        slot: 0,
        token: 53,
        born_revision: 1,
        kind: ModuleKind::StockVco,
        bypassed: false,
        action_epoch: 1,
        parameter_epochs: [0; MAX_PUBLISHED_PARAMETERS],
    };
    let meter = Meter::new();
    assert!(meter.read(entry, 1).is_none());
    for present in [true, false] {
        let mut lights = ModuleLightState {
            count: 5,
            clock_frequency: Some(2.0),
            ..Default::default()
        };
        lights.brightness[0][0] = -0.01;
        lights.brightness[1][0] = 0.5;
        lights.brightness[3][0] = 1.0;
        meter.write(
            entry.token,
            ModuleSnapshot {
                parameters: [None; MAX_PUBLISHED_PARAMETERS],
                revision: 7,
                frame: 512,
                state: None,
                saved_state: None,
                controls: None,
                midi_learning: None,
                lights: present.then_some(lights),
            },
        );
        let packet = meter.read(entry, 7).unwrap();
        assert_eq!(packet.lights.is_some(), present);
        assert!(packet.state.is_none() && packet.controls.is_none());
        if let Some(actual) = packet.lights {
            assert_eq!(actual.count, 5);
            assert_eq!(actual.brightness, lights.brightness);
            assert_eq!(
                actual.clock_frequency, None,
                "Stock VCO cannot publish a foreign clock observation"
            );
        }
        assert!(meter.read(entry, 8).is_none());
        assert!(meter.read(Entry { token: 54, ..entry }, 7).is_none());
    }
}

#[test]
fn rack264_vco_keeps_its_own_passive_runtime_light_identity() {
    assert!(ModuleKind::from_name("rack264_vco") == Some(ModuleKind::Rack264Vco));
    assert_eq!(ModuleKind::Rack264Vco.name(), "rack264_vco");
    assert!(!ModuleKind::Rack264Vco.manual() && !ModuleKind::Rack264Vco.sequential());
    assert!(!ModuleKind::Rack264Vco.accepts(ModuleAction::BPress));
    let entry = Entry {
        slot: 0,
        token: 89,
        born_revision: 1,
        kind: ModuleKind::Rack264Vco,
        bypassed: false,
        action_epoch: 1,
        parameter_epochs: [0; MAX_PUBLISHED_PARAMETERS],
    };
    let mut lamps = ModuleLightState {
        count: 5,
        ..Default::default()
    };
    lamps.brightness[1][0] = 0.4;
    lamps.brightness[3][0] = 1.0;
    let meter = Meter::new();
    meter.write(
        entry.token,
        ModuleSnapshot {
            parameters: [None; MAX_PUBLISHED_PARAMETERS],
            revision: 7,
            frame: 512,
            state: None,
            saved_state: None,
            controls: None,
            midi_learning: None,
            lights: Some(lamps),
        },
    );
    let actual = meter.read(entry, 7).unwrap().lights.unwrap();
    assert_eq!(actual.count, 5);
    assert_eq!(actual.brightness, lamps.brightness);
    assert_eq!(actual.clock_frequency, None);
    assert!(meter.read(entry, 8).is_none());
}
