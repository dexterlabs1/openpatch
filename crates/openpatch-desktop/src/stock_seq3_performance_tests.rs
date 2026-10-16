#[test]
fn stock_seq3_meter_preserves_all_durable_bits_and_copublishes_controls_telemetry_and_lights() {
    assert!(ModuleKind::StockSeq3.manual() && ModuleKind::StockSeq3.sequential());
    assert!(ModuleKind::from_name("stock_seq3") == Some(ModuleKind::StockSeq3));
    for action in [
        ModuleAction::Clock,
        ModuleAction::ClockPress,
        ModuleAction::ClockRelease,
        ModuleAction::RunPress,
        ModuleAction::RunRelease,
        ModuleAction::ResetPress,
        ModuleAction::ResetRelease,
        ModuleAction::SeqGatePress(1),
        ModuleAction::SeqGatePress(8),
        ModuleAction::SeqGateRelease(8),
    ] {
        assert!(ModuleKind::StockSeq3.accepts(action));
    }
    for action in [
        ModuleAction::SeqGatePress(0),
        ModuleAction::SeqGatePress(9),
        ModuleAction::SeqGateRelease(0),
        ModuleAction::BPress,
    ] {
        assert!(!ModuleKind::StockSeq3.accepts(action));
    }
    for action in [
        ModuleAction::ClockRelease,
        ModuleAction::RunRelease,
        ModuleAction::ResetRelease,
        ModuleAction::SeqGateRelease(8),
    ] {
        assert!(is_release(action));
    }
    assert!(!ModuleKind::Sequence.manual());
    assert!(!ModuleKind::Sequence.accepts(ModuleAction::ClockPress));
    let entry = Entry {
        slot: 0,
        token: 53,
        born_revision: 1,
        kind: ModuleKind::StockSeq3,
        bypassed: false,
        action_epoch: 1,
    };
    let meter = Meter::new();
    assert!(meter.read(entry, 1).is_none());
    for running in [false, true] {
        for passthrough in [false, true] {
            for gates in 0..=255_u8 {
                let saved = SavedModuleState::StockSeq3 {
                    running,
                    gates,
                    passthrough,
                };
                let mut lights = ModuleLightState {
                    count: 19,
                    ..Default::default()
                };
                for (i, pair) in lights.brightness[..19].iter_mut().enumerate() {
                    *pair = [i as f32 / 19.0, gates as f32 / 255.0];
                }
                let controls = ModuleControlState {
                    pressed: true,
                    pressed_mask: 0x7ff,
                };
                let state = ModuleTelemetry {
                    step: gates % 8 + 1,
                    length: 8,
                    running,
                };
                meter.write(
                    entry.token,
                    ModuleSnapshot {
                        revision: 7,
                        frame: u64::from(gates),
                        state: Some(state),
                        saved_state: Some(saved),
                        controls: Some(controls),
                        midi_learning: None,
                        lights: Some(lights),
                    },
                );
                let packet = meter.read(entry, 7).unwrap();
                assert_eq!(packet.saved_state, Some(saved));
                assert_eq!(packet.controls, Some(controls));
                assert_eq!(packet.state, Some(state));
                assert_eq!(packet.lights, Some(lights));
                assert!(meter.read(entry, 8).is_none());
                assert!(meter.read(Entry { token: 54, ..entry }, 7).is_none());
            }
        }
    }
    meter.write(
        entry.token,
        ModuleSnapshot {
            revision: 8,
            frame: 1000,
            state: None,
            saved_state: None,
            controls: None,
            midi_learning: None,
            lights: None,
        },
    );
    let packet = meter.read(entry, 8).unwrap();
    assert!(
        packet.state.is_none()
            && packet.saved_state.is_none()
            && packet.controls.is_none()
            && packet.lights.is_none()
    );
}

#[test]
fn stock_seq3_snapshot_cannot_tear_full_flags_controls_or_nineteen_light_groups() {
    let (performance, _, _) = Performance::new(48000);
    {
        let mut control = performance.control.lock().unwrap();
        control.revision = 1;
        control.entries.insert(
            "seq".into(),
            Entry {
                slot: 0,
                token: 53,
                born_revision: 1,
                kind: ModuleKind::StockSeq3,
                bypassed: false,
                action_epoch: 1,
            },
        );
    }
    let shared = performance.shared.clone();
    let writer = std::thread::spawn(move || {
        for frame in 1..=1500_u64 {
            let gates = frame as u8;
            let mut lights = ModuleLightState {
                count: 19,
                ..Default::default()
            };
            lights.brightness[18] = [f32::from(gates), frame as f32];
            shared.publish_frame(frame, || {
                shared.meters[0].write(
                    53,
                    ModuleSnapshot {
                        revision: frame + 1,
                        frame,
                        state: Some(ModuleTelemetry {
                            step: gates % 8 + 1,
                            length: 8,
                            running: gates & 1 != 0,
                        }),
                        saved_state: Some(SavedModuleState::StockSeq3 {
                            running: gates & 1 != 0,
                            gates,
                            passthrough: gates & 2 != 0,
                        }),
                        controls: Some(ModuleControlState {
                            pressed: gates != 0,
                            pressed_mask: u16::from(gates),
                        }),
                        midi_learning: None,
                        lights: Some(lights),
                    },
                )
            });
        }
    });
    let check = |snapshot: Value| {
        let seq = &snapshot["modules"]["seq"];
        if seq["available"] != true {
            assert!(seq["state"].is_null() && seq["controls"].is_null() && seq["lights"].is_null());
            return;
        }
        let frame = snapshot["frame"].as_u64().unwrap();
        let gates = frame as u8;
        assert_eq!(seq["frame"], frame);
        assert_eq!(seq["revision"], frame + 1);
        assert_eq!(seq["state"]["running"], gates & 1 != 0);
        assert_eq!(seq["state"]["passthrough"], gates & 2 != 0);
        assert_eq!(
            seq["state"]["gates"],
            json!(std::array::from_fn::<_, 8, _>(|i| gates & (1 << i) != 0))
        );
        assert_eq!(seq["state"]["step"], gates % 8 + 1);
        assert_eq!(seq["state"]["length"], 8);
        assert_eq!(seq["controls"]["pressed_mask"], gates);
        assert_eq!(seq["lights"]["count"], 19);
        assert_eq!(
            seq["lights"]["brightness"][18],
            json!([f32::from(gates), frame as f32])
        );
    };
    for _ in 0..1500 {
        check(performance.snapshot());
    }
    writer.join().unwrap();
    check(performance.snapshot());
    performance.close();
    check(performance.snapshot());
}
#[test]
fn stock_seq3_selective_state_tag_eight_roundtrips_every_optional_flag_and_gate_mask() {
    let entry = Entry {
        slot: 0,
        token: 4,
        born_revision: 1,
        kind: ModuleKind::StockSeq3,
        bypassed: false,
        action_epoch: 1,
    };
    let meter = Meter::new();
    for running in [None, Some(false), Some(true)] {
        for passthrough in [None, Some(false), Some(true)] {
            for gate_mask in 0..=255 {
                let value = SavedModuleState::StockSeq3Fields {
                    running,
                    gates: !gate_mask,
                    gate_mask,
                    passthrough,
                };
                meter.write(
                    entry.token,
                    ModuleSnapshot {
                        revision: 1,
                        frame: 1,
                        state: None,
                        saved_state: Some(value),
                        controls: None,
                        lights: None,
                        midi_learning: None,
                    },
                );
                assert_eq!(meter.read(entry, 1).unwrap().saved_state, Some(value));
            }
        }
    }
    for value in [
        SavedModuleState::Seq3Run(true),
        SavedModuleState::PushHold(true),
        SavedModuleState::StockSeq3 {
            running: true,
            gates: 255,
            passthrough: true,
        },
    ] {
        meter.write(
            entry.token,
            ModuleSnapshot {
                revision: 1,
                frame: 1,
                state: None,
                saved_state: Some(value),
                controls: None,
                lights: None,
                midi_learning: None,
            },
        );
        assert_eq!(meter.read(entry, 1).unwrap().saved_state, Some(value));
    }
}

#[test]
fn performance_wire_lights_publish_logical_groups_instead_of_callback_storage_capacity() {
    let (performance, _, _) = Performance::new(48000);
    {
        let mut control = performance.control.lock().unwrap();
        control.revision = 1;
        control.entries.insert(
            "lights".into(),
            Entry {
                slot: 0,
                token: 47,
                born_revision: 1,
                kind: ModuleKind::StockSeq3,
                bypassed: false,
                action_epoch: 1,
            },
        );
    }
    for count in [
        0,
        5,
        16,
        19,
        MAX_MODULE_LIGHTS as u8,
        MAX_MODULE_LIGHTS as u8 + 1,
        u8::MAX,
    ] {
        let mut lights = ModuleLightState {
            count,
            manual: true,
            input_channels: Some(0),
            ..Default::default()
        };
        for (index, pair) in lights.brightness.iter_mut().enumerate() {
            *pair = [index as f32, -(index as f32)];
        }
        let frame = u64::from(count) + 1;
        performance.shared.publish_frame(frame, || {
            performance.shared.meters[0].write(
                47,
                ModuleSnapshot {
                    revision: 1,
                    frame,
                    state: None,
                    saved_state: None,
                    controls: None,
                    lights: Some(lights),
                    midi_learning: None,
                },
            )
        });
        let snapshot = performance.snapshot();
        let module = &snapshot["modules"]["lights"];
        assert_eq!(module["available"], true);
        let wire = &module["lights"];
        if usize::from(count) > MAX_MODULE_LIGHTS {
            assert!(
                wire.is_null(),
                "Invalid count cannot expose a truncated but inconsistent packet"
            );
            continue;
        }
        assert_eq!(wire["count"], count);
        assert_eq!(
            wire["brightness"].as_array().unwrap().len(),
            usize::from(count)
        );
        assert_eq!(wire["manual"], true);
        assert_eq!(wire["input_channels"], 0);
        assert!(wire.get("clock_frequency").is_none());
        for index in 0..usize::from(count) {
            assert_eq!(wire["brightness"][index], json!(lights.brightness[index]));
        }
        if count == 19 {
            assert_eq!(
                wire["brightness"][18],
                json!([18.0, -18.0]),
                "Stock SEQ3's final yellow/red group survives intact"
            );
        }
        let typed = performance.module_lights("lights").unwrap();
        assert_eq!(
            typed.brightness, lights.brightness,
            "Wire projection must not mutate callback storage"
        );
    }
}
