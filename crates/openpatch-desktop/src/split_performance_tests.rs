fn split_packet(revision: u64, frame: u64, channels: Option<u8>, present: bool) -> ModuleSnapshot {
    ModuleSnapshot { parameters: [None; MAX_PUBLISHED_PARAMETERS],
        revision, frame, state: None, saved_state: None, controls: None, midi_learning: None,
        lights: present.then_some(ModuleLightState { input_channels: channels, count: 0, ..Default::default() }),
    }
}

#[test]
fn split_is_passive_and_optional_count_packets_respect_revision_and_slot_identity() {
    assert!(ModuleKind::from_name("poly_split") == Some(ModuleKind::Split));
    assert_eq!(ModuleKind::Split.name(), "poly_split");
    assert!(!ModuleKind::Split.manual());
    assert!(!ModuleKind::Split.sequential());
    assert!(!ModuleKind::Split.accepts(ModuleAction::BPress));
    assert!(!ModuleKind::Split.has_safety_release());
    let meter = Meter::new();
    let entry = Entry { slot: 0, token: 17, born_revision: 2, kind: ModuleKind::Split, bypassed: false, action_epoch: 1 };
    assert!(meter.read(entry, 2).is_none());
    for count in [Some(16), Some(0), None, Some(1), Some(17), Some(255)] {
        for present in [true, false] {
            meter.write(entry.token, split_packet(7, 101, count, present));
            let actual = meter.read(entry, 7).unwrap();
            assert_eq!(actual.lights.is_some(), present);
            assert_eq!(actual.lights.and_then(|lights| lights.input_channels), if present { count.filter(|count| *count <= 16) } else { None });
            if let Some(lights) = actual.lights {
                assert_eq!(lights.count, 0);
                assert!(!lights.manual);
                assert_eq!(lights.brightness, [[0.0; 2]; MAX_MODULE_LIGHTS]);
                assert_eq!(lights.output_channels, None);
                assert_eq!(lights.automatic_channels, None);
                assert_eq!(lights.waveform_position, None);
            }
            assert!(actual.state.is_none());
            assert!(actual.controls.is_none());
            assert!(meter.read(entry, 8).is_none());
            assert!(meter.read(Entry { token: 18, ..entry }, 7).is_none());
        }
    }
    // Slot reuse is unavailable until a packet from the new generation arrives.
    meter.write(18, split_packet(8, 102, Some(0), true));
    assert!(meter.read(entry, 7).is_none());
    assert_eq!(meter.read(Entry { token: 18, ..entry }, 8).unwrap().lights.unwrap().input_channels, Some(0));
}

#[test]
fn split_count_packets_never_mix_frame_revision_or_instance_generation() {
    let meter = Arc::new(Meter::new());
    let entry = Entry { slot: 0, token: 17, born_revision: 1, kind: ModuleKind::Split, bypassed: false, action_epoch: 1 };
    let writer_meter = meter.clone();
    let writer = std::thread::spawn(move || {
        for frame in 1..=1500_u64 {
            writer_meter.write(17 + frame % 2, split_packet(frame + 1, frame, Some((frame % 17) as u8), !frame.is_multiple_of(7)));
            std::thread::yield_now();
        }
    });
    for _ in 0..1500 {
        for token in [17, 18] {
            if let Some(packet) = meter.read(Entry { token, ..entry }, 1) {
                assert_eq!(packet.revision, packet.frame + 1);
                assert_eq!(token, 17 + packet.frame % 2);
                assert_eq!(packet.lights.is_some(), !packet.frame.is_multiple_of(7));
                if let Some(lights) = packet.lights {
                    assert_eq!(lights.input_channels, Some((packet.frame % 17) as u8));
                    assert_eq!(lights.count, 0);
                    assert_eq!(lights.brightness, [[0.0; 2]; MAX_MODULE_LIGHTS]);
                }
            }
        }
        std::thread::yield_now();
    }
    writer.join().unwrap();
}
