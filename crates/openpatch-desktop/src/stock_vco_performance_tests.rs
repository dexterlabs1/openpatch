#[test]
fn stock_vco_is_passive_and_five_lamps_obey_packet_identity_without_clock_metadata() {
    assert!(ModuleKind::from_name("stock_vco") == Some(ModuleKind::StockVco));
    assert_eq!(ModuleKind::StockVco.name(), "stock_vco");
    assert!(!ModuleKind::StockVco.manual() && !ModuleKind::StockVco.sequential());
    assert!(!ModuleKind::StockVco.accepts(ModuleAction::BPress));
    let entry = Entry { slot: 0, token: 53, born_revision: 1, kind: ModuleKind::StockVco, bypassed: false, action_epoch: 1 };
    let meter = Meter::new();
    assert!(meter.read(entry, 1).is_none());
    for present in [true, false] {
        let mut lights = ModuleLightState { count: 5, clock_frequency: Some(2.0), ..Default::default() };
        lights.brightness[0][0] = -0.01; lights.brightness[1][0] = 0.5; lights.brightness[3][0] = 1.0;
        meter.write(entry.token, ModuleSnapshot { revision: 7, frame: 512, state: None, saved_state: None, controls: None, midi_learning: None, lights: present.then_some(lights) });
        let packet = meter.read(entry, 7).unwrap(); assert_eq!(packet.lights.is_some(), present);
        assert!(packet.state.is_none() && packet.controls.is_none());
        if let Some(actual) = packet.lights {
            assert_eq!(actual.count, 5); assert_eq!(actual.brightness, lights.brightness);
            assert_eq!(actual.clock_frequency, None, "Stock VCO cannot publish a foreign clock observation");
        }
        assert!(meter.read(entry, 8).is_none()); assert!(meter.read(Entry { token: 54, ..entry }, 7).is_none());
    }
}
