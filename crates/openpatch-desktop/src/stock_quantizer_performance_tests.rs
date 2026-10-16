fn quantizer_entry() -> Entry {
    Entry {
        slot: 0,
        token: 71,
        born_revision: 1,
        kind: ModuleKind::StockQuantizer,
        bypassed: false,
        action_epoch: 1,
    }
}
#[test]
fn stock_quantizer_atomic_packets_distinguish_none_zero_and_valid_banks_and_clear_foreign_identity()
{
    assert!(ModuleKind::from_name("stock_quantizer") == Some(ModuleKind::StockQuantizer));
    assert!(!ModuleKind::StockQuantizer.manual() && !ModuleKind::StockQuantizer.sequential());
    assert!(!ModuleKind::StockQuantizer.accepts(ModuleAction::Clock));
    let meter = Meter::new();
    let entry = quantizer_entry();
    assert!(meter.read(entry, 1).is_none());
    for notes in [
        None,
        Some(0),
        Some(1),
        Some(2048),
        Some(4095),
        Some(4096),
        Some(u16::MAX),
    ] {
        for present in [false, true] {
            let lights = ModuleLightState {
                count: 0,
                playing_notes: notes,
                ..Default::default()
            };
            meter.write(
                entry.token,
                ModuleSnapshot {
                    revision: 7,
                    frame: 1024,
                    state: None,
                    saved_state: Some(SavedModuleState::StockQuantizerNotes(2741)),
                    controls: None,
                    lights: present.then_some(lights),
                    midi_learning: None,
                },
            );
            let packet = meter.read(entry, 7).unwrap();
            assert_eq!(packet.lights.is_some(), present);
            if let Some(lights) = packet.lights {
                assert_eq!(lights.playing_notes, notes.filter(|mask| *mask <= 4095));
                assert_eq!(lights.count, 0);
                assert_eq!(lights.brightness, [[0.0; 2]; MAX_MODULE_LIGHTS]);
            }
            assert_eq!(
                packet.saved_state,
                Some(SavedModuleState::StockQuantizerNotes(2741))
            );
            assert!(meter.read(entry, 8).is_none());
            assert!(meter.read(Entry { token: 72, ..entry }, 7).is_none());
            let foreign = meter
                .read(
                    Entry {
                        kind: ModuleKind::StockVco,
                        ..entry
                    },
                    7,
                )
                .unwrap();
            assert_eq!(foreign.lights.and_then(|lights| lights.playing_notes), None);
        }
    }
    for mask in 0..=4095 {
        meter.write(
            entry.token,
            ModuleSnapshot {
                revision: 8,
                frame: 2048,
                state: None,
                saved_state: Some(SavedModuleState::StockQuantizerNotes(mask)),
                controls: None,
                lights: None,
                midi_learning: None,
            },
        );
        assert_eq!(
            meter.read(entry, 8).unwrap().saved_state,
            Some(SavedModuleState::StockQuantizerNotes(mask))
        );
    }
}
#[test]
fn stock_quantizer_snapshot_bank_playing_union_and_empty_lamp_array_share_one_serial_boundary() {
    let (performance, _, _) = Performance::new(48000);
    {
        let mut control = performance.control.lock().unwrap();
        control.revision = 1;
        control.entries.insert("q".into(), quantizer_entry());
    }
    let shared = performance.shared.clone();
    let writer = std::thread::spawn(move || {
        for frame in 1..=1500_u64 {
            let mask = (frame % 4096) as u16;
            shared.publish_frame(frame, || {
                shared.meters[0].write(
                    71,
                    ModuleSnapshot {
                        revision: frame + 1,
                        frame,
                        state: None,
                        saved_state: Some(SavedModuleState::StockQuantizerNotes(mask)),
                        controls: None,
                        midi_learning: None,
                        lights: Some(ModuleLightState {
                            count: 0,
                            playing_notes: (!frame.is_multiple_of(7)).then_some(mask ^ 4095),
                            ..Default::default()
                        }),
                    },
                )
            });
        }
    });
    let check = |snapshot: Value| {
        let q = &snapshot["modules"]["q"];
        if q["available"] != true {
            assert!(q["state"].is_null() && q["lights"].is_null());
            return;
        }
        let frame = snapshot["frame"].as_u64().unwrap();
        let mask = (frame % 4096) as u16;
        assert_eq!(q["revision"], frame + 1);
        assert_eq!(q["frame"], frame);
        assert_eq!(q["state"]["scale_mask"], mask);
        assert_eq!(
            q["state"]["enabled_notes"],
            json!(std::array::from_fn::<_, 12, _>(|i| mask & (1 << i) != 0))
        );
        assert_eq!(q["lights"]["count"], 0);
        assert_eq!(q["lights"]["brightness"], json!([]));
        if frame.is_multiple_of(7) {
            assert!(q["lights"].get("playing_notes").is_none());
        } else {
            assert_eq!(q["lights"]["playing_notes"], mask ^ 4095);
        }
        assert!(q["controls"].is_null());
    };
    for _ in 0..1500 {
        check(performance.snapshot());
    }
    writer.join().unwrap();
    check(performance.snapshot());
    performance.close();
    check(performance.snapshot());
}
