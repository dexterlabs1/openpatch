fn stock_octave_packet(revision: u64, frame: u64, lights: Option<ModuleLightState>) -> ModuleSnapshot {
    ModuleSnapshot { parameters: [None; MAX_PUBLISHED_PARAMETERS],
        revision,
        frame,
        state: None,
        saved_state: None,
        controls: None,
        lights,
        midi_learning: None,
    }
}

fn stock_octave_entry(token: u64) -> Entry {
    Entry {
        slot: 0,
        token,
        born_revision: 1,
        kind: ModuleKind::StockOctave,
        bypassed: false,
        action_epoch: 1,
        parameter_epochs: [0; MAX_PUBLISHED_PARAMETERS],
    }
}

#[test]
fn stock_octave_meter_round_trips_optional_signed_extremes_and_clears_stale_values() {
    assert!(ModuleKind::from_name("stock_octave") == Some(ModuleKind::StockOctave));
    assert_eq!(ModuleKind::StockOctave.name(), "stock_octave");
    assert!(!ModuleKind::StockOctave.manual() && !ModuleKind::StockOctave.sequential());
    let entry = stock_octave_entry(91);
    let meter = Meter::new();
    assert!(meter.read(entry, 1).is_none());

    for (frame, shift) in [None, Some(0), Some(1), Some(-1), Some(i32::MIN), Some(i32::MAX)]
        .into_iter()
        .enumerate()
    {
        let lights = ModuleLightState {
            octave_shift: shift,
            ..Default::default()
        };
        meter.write(
            entry.token,
            stock_octave_packet(7, frame as u64, Some(lights)),
        );
        let packet = meter.read(entry, 7).expect("matching token and revision");
        assert_eq!(packet.frame, frame as u64);
        assert_eq!(packet.lights.unwrap().octave_shift, shift);
        assert!(meter.read(entry, 8).is_none(), "future revision is rejected");
        assert!(meter.read(stock_octave_entry(92), 7).is_none(), "foreign token is rejected");
    }

    meter.write(
        entry.token,
        stock_octave_packet(
            8,
            100,
            Some(ModuleLightState {
                octave_shift: Some(i32::MAX),
                ..Default::default()
            }),
        ),
    );
    meter.write(entry.token, stock_octave_packet(9, 101, None));
    assert!(meter.read(entry, 9).unwrap().lights.is_none(), "absent lights clear prior shift telemetry");

    meter.write(
        entry.token,
        stock_octave_packet(
            10,
            102,
            Some(ModuleLightState {
                octave_shift: Some(i32::MIN),
                ..Default::default()
            }),
        ),
    );
    let foreign_kind = Entry {
        kind: ModuleKind::StockVco,
        ..entry
    };
    assert_eq!(
        meter
            .read(foreign_kind, 10)
            .unwrap()
            .lights
            .and_then(|lights| lights.octave_shift),
        None,
        "only Stock Octave exposes the encoded signed field"
    );
}

#[test]
fn stock_octave_meter_serial_keeps_token_revision_frame_and_optional_shift_coherent() {
    use std::sync::Arc;

    let meter = Arc::new(Meter::new());
    let writer_meter = meter.clone();
    let writer = std::thread::spawn(move || {
        for frame in 1..=2500_u64 {
            let token = 17 + frame % 2;
            let lights = (frame % 7 != 0).then_some(ModuleLightState {
                octave_shift: Some(frame as i32),
                ..Default::default()
            });
            writer_meter.write(token, stock_octave_packet(frame + 1, frame, lights));
            std::thread::yield_now();
        }
    });

    for _ in 0..2500 {
        for token in [17, 18] {
            if let Some(packet) = meter.read(stock_octave_entry(token), 1) {
                assert_eq!(packet.revision, packet.frame + 1);
                assert_eq!(token, 17 + packet.frame % 2);
                assert_eq!(packet.lights.is_some(), !packet.frame.is_multiple_of(7));
                if let Some(lights) = packet.lights {
                    assert_eq!(lights.octave_shift, Some(packet.frame as i32));
                }
            }
        }
        std::thread::yield_now();
    }
    writer.join().unwrap();
}
