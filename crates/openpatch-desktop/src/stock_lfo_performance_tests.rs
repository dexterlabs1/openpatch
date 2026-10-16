fn stock_lfo_packet(frame: u64, clock_frequency: Option<f32>, present: bool) -> ModuleSnapshot {
    ModuleSnapshot { revision: frame + 1, frame, state: None, saved_state: None, controls: None, midi_learning: None,
        lights: present.then_some(ModuleLightState { count: 5, clock_frequency, ..Default::default() }) }
}
#[test]
fn stock_lfo_clock_transport_clears_absent_invalid_foreign_and_reused_packets() {
    assert!(ModuleKind::from_name("stock_lfo") == Some(ModuleKind::StockLfo));
    assert_eq!(ModuleKind::StockLfo.name(), "stock_lfo");
    assert!(!ModuleKind::StockLfo.manual() && !ModuleKind::StockLfo.sequential());
    assert!(!ModuleKind::StockLfo.accepts(ModuleAction::BPress));
    let meter = Meter::new();
    let entry = Entry { slot: 0, token: 41, born_revision: 1, kind: ModuleKind::StockLfo, bypassed: false, action_epoch: 1 };
    for clock in [Some(2.0), Some(1.0), None, Some(0.0), Some(-1.0), Some(f32::NAN), Some(f32::INFINITY), Some(f32::MAX)] {
        for present in [true, false] {
            meter.write(entry.token, stock_lfo_packet(7, clock, present));
            let expected = clock.filter(|value| value.is_finite() && *value > 0.0).filter(|_| present);
            let packet = meter.read(entry, 8).unwrap();
            assert_eq!(packet.lights.and_then(|lights| lights.clock_frequency), expected);
            let raw = f32::from_bits(meter.clock_frequency.load(Ordering::SeqCst));
            if let Some(expected) = expected { assert_eq!(raw, expected); } else { assert!(raw.is_nan()); }
            assert_eq!(meter.read(Entry { kind: ModuleKind::WavetableLfo, ..entry }, 8).unwrap().lights.and_then(|lights| lights.clock_frequency), None);
            assert!(meter.read(entry, 9).is_none());
            assert!(meter.read(Entry { token: 42, ..entry }, 8).is_none());
        }
    }
    meter.write(42, stock_lfo_packet(8, Some(1.0), true));
    assert!(meter.read(entry, 8).is_none());
    assert_eq!(meter.read(Entry { token: 42, ..entry }, 9).unwrap().lights.unwrap().clock_frequency, Some(1.0));
}
#[test]
fn stock_lfo_clock_and_five_lamps_are_one_coherent_frame_and_generation() {
    let meter = Arc::new(Meter::new());
    let entry = Entry { slot: 0, token: 41, born_revision: 1, kind: ModuleKind::StockLfo, bypassed: false, action_epoch: 1 };
    let writer_meter = meter.clone();
    let writer = std::thread::spawn(move || {
        for frame in 1..=1500_u64 {
            let mut packet = stock_lfo_packet(frame, Some(frame as f32), !frame.is_multiple_of(7));
            if let Some(lights) = &mut packet.lights {
                for (i, row) in lights.brightness.iter_mut().take(5).enumerate() { row[0] = (frame * 5 + i as u64) as f32; }
            }
            writer_meter.write(41 + frame % 2, packet);
            std::thread::yield_now();
        }
    });
    for _ in 0..1500 {
        for token in [41, 42] {
            if let Some(packet) = meter.read(Entry { token, ..entry }, 1) {
                assert_eq!(packet.revision, packet.frame + 1);
                assert_eq!(token, 41 + packet.frame % 2);
                assert_eq!(packet.lights.is_some(), !packet.frame.is_multiple_of(7));
                if let Some(lights) = packet.lights {
                    assert_eq!(lights.clock_frequency, Some(packet.frame as f32));
                    assert_eq!(lights.count, 5);
                    for i in 0..5 { assert_eq!(lights.brightness[i][0], (packet.frame * 5 + i as u64) as f32); }
                }
            }
        }
        std::thread::yield_now();
    }
    writer.join().unwrap();
}
