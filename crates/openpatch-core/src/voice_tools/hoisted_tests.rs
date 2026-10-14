use super::scalar_reference::VoiceTools as ScalarTools;
use super::*;

const DRIVES: [f32; 5] = [0.0, 1.0e-12, f32::EPSILON, 0.1, 1.0];

fn node(drive: f32, revision: usize) -> Node {
    Node {
        score: None,
        id: "hoisted-test".into(),
        kind: "svf_filter".into(),
        bypassed: false,
        params: [
            ("drive", drive),
            ("cutoff", [5.0, 731.0, 17000.0][revision % 3]),
            ("resonance", [0.0, 0.37, 1.0][revision % 3]),
            ("cutoff_cv_amount", [-4.0, 0.7, 4.0][revision % 3]),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect(),
    }
}

fn frames(count: usize, frame: usize) -> [PortFrame; 2] {
    let mut signal = PortFrame {
        channels: count as u8,
        ..Default::default()
    };
    let mut cv = PortFrame {
        channels: [0, 1, 16, 5][(frame / 37) % 4],
        ..Default::default()
    };
    for lane in 0..MAX_CHANNELS {
        signal.values[lane] = match (frame + lane * 7) % 113 {
            0 => -0.0,
            1 => 0.0,
            2 => f32::NAN,
            3 => f32::INFINITY,
            4 => f32::NEG_INFINITY,
            5 => 2.0e6,
            6 => -2.0e6,
            _ => {
                ((frame as f64 * (0.137 + lane as f64 * 0.011)).sin() * (5.0 + lane as f64 * 0.1))
                    as f32
            }
        };
        cv.values[lane] = match (frame + lane) % 97 {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => -100.0,
            3 => 100.0,
            _ => ((frame as f64 * 0.071 + lane as f64 * 0.3).cos() * 2.0) as f32,
        };
    }
    [signal, cv]
}

fn compare(
    hoisted: &mut VoiceTools,
    scalar: &mut ScalarTools,
    inputs: &[PortFrame],
    context: &str,
) {
    // Fill with poison so full-port comparison also checks inactive clearing.
    let poison = PortFrame {
        channels: 16,
        values: [f32::NAN; MAX_CHANNELS],
    };
    let mut actual = [poison; 3];
    let mut expected = [poison; 3];
    hoisted.process(inputs, &mut actual);
    scalar.process(inputs, &mut expected);
    for port in 0..3 {
        assert_eq!(
            actual[port].channels, expected[port].channels,
            "{context}: port {port} count"
        );
        for lane in 0..MAX_CHANNELS {
            assert_eq!(
                actual[port].values[lane].to_bits(),
                expected[port].values[lane].to_bits(),
                "{context}: port {port}, lane {lane}: actual {:?}, scalar {:?}",
                actual[port].values[lane],
                expected[port].values[lane]
            );
        }
    }
}

fn filter(tool: &VoiceTools) -> &Filter {
    let VoiceTools::Filter(filter) = tool else {
        panic!("expected filter")
    };
    filter
}

fn assert_same_state(hoisted: &VoiceTools, scalar: &ScalarTools) {
    // The immutable oracle's fields stay private. Debug prints every f64 with
    // round-trip precision and preserves signed zero; the common state suffix
    // includes both SVF states, every FIR slot/head, all CV slots/head and active.
    // Compiled parameters preceding this suffix intentionally differ in shape.
    let actual = format!("{hoisted:?}");
    let expected = format!("{scalar:?}");
    assert_eq!(
        actual.split_once("states: ").unwrap().1,
        expected.split_once("states: ").unwrap().1
    );
}

fn assert_inactive_clear(tool: &VoiceTools) {
    let f = filter(tool);
    let pristine = format!("{:?}", DriveLane::new());
    for lane in f.active..MAX_CHANNELS {
        assert_eq!(
            f.states[lane].map(f64::to_bits),
            [0; 2],
            "inactive SVF lane {lane}"
        );
        assert_eq!(
            format!("{:?}", f.drive_lanes[lane]),
            pristine,
            "inactive FIR/CV lane {lane}"
        );
    }
}

#[test]
fn all_counts_drives_rates_and_three_outputs_match_scalar_bits() {
    for rate in [44100, 48000, 96000] {
        for drive in DRIVES {
            for count in 1..=MAX_CHANNELS {
                let n = node(drive, count);
                let mut hoisted = VoiceTools::build(&n, rate).unwrap();
                let mut scalar = ScalarTools::build(&n, rate).unwrap();
                for frame in 0..160 {
                    compare(
                        &mut hoisted,
                        &mut scalar,
                        &frames(count, frame),
                        &format!("rate {rate}, drive {drive}, count {count}, frame {frame}"),
                    );
                }
                // Continue for >76 host frames after the stimulus to expose all
                // retained interpolation, decimation, CV and filter histories.
                let silence = [PortFrame {
                    channels: count as u8,
                    ..Default::default()
                }; 2];
                for frame in 0..192 {
                    compare(
                        &mut hoisted,
                        &mut scalar,
                        &silence,
                        &format!("tail count {count} frame {frame}"),
                    );
                }
                assert_inactive_clear(&hoisted);
                assert_same_state(&hoisted, &scalar);
            }
        }
    }
}

#[test]
fn changing_counts_keep_different_heads_and_clear_retired_lanes() {
    for drive in DRIVES {
        let n = node(drive, 1);
        let mut hoisted = VoiceTools::build(&n, 48000).unwrap();
        let mut scalar = ScalarTools::build(&n, 48000).unwrap();
        let counts = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 5, 11, 2, 16, 0, 17, 3, 8,
        ];
        for (segment, count) in counts.into_iter().enumerate() {
            for frame in 0..(17 + segment % 13) {
                compare(
                    &mut hoisted,
                    &mut scalar,
                    &frames(count, segment * 53 + frame),
                    &format!(
                        "drive {drive}, changing count {count}, segment {segment}, frame {frame}"
                    ),
                );
            }
            assert_inactive_clear(&hoisted);
            assert_same_state(&hoisted, &scalar);
            if segment == 15 {
                // Sequential activation creates differing CV ring heads inside
                // each lane. FIR heads also differ because they advance with
                // each lane's independently activated host-frame count.
                let f = filter(&hoisted);
                assert_ne!(f.drive_lanes[0].head, f.drive_lanes[1].head);
                assert_ne!(
                    format!("{:?}", f.drive_lanes[0].oversampler),
                    format!("{:?}", f.drive_lanes[1].oversampler)
                );
            }
        }
        for frame in 0..160 {
            compare(
                &mut hoisted,
                &mut scalar,
                &frames(16, 3000 + frame),
                "count-change continuation",
            );
        }
    }
}

#[test]
fn reset_and_adoption_keep_new_compiled_drive_constants_and_scalar_continuation() {
    let mut hoisted = VoiceTools::build(&node(0.0, 0), 48000).unwrap();
    let mut scalar = ScalarTools::build(&node(0.0, 0), 48000).unwrap();
    for (revision, drive) in [0.0, 1.0, 1.0e-12, 0.1, 0.0, 1.0].into_iter().enumerate() {
        let n = node(drive, revision);
        let mut next_hoisted = VoiceTools::build(&n, 48000).unwrap();
        let mut next_scalar = ScalarTools::build(&n, 48000).unwrap();
        let compiled = (
            filter(&next_hoisted).gain,
            filter(&next_hoisted).blend,
            filter(&next_hoisted).drive_denom,
        );
        next_hoisted.adopt(&hoisted);
        next_scalar.adopt(&scalar);
        assert_eq!(
            compiled,
            (
                filter(&next_hoisted).gain,
                filter(&next_hoisted).blend,
                filter(&next_hoisted).drive_denom
            )
        );
        assert_same_state(&next_hoisted, &next_scalar);
        hoisted = next_hoisted;
        scalar = next_scalar;
        for frame in 0..240 {
            compare(
                &mut hoisted,
                &mut scalar,
                &frames([16, 3, 13, 8][(frame / 53) % 4], revision * 241 + frame),
                &format!("adopt revision {revision}, drive {drive}, frame {frame}"),
            );
            if frame == 119 {
                hoisted.reset_state();
                scalar.reset_state();
                assert_inactive_clear(&hoisted);
                assert_same_state(&hoisted, &scalar);
            }
        }
    }
    // The scalar prototype does not adopt across sample-rate changes.
    let mut different_rate = VoiceTools::build(&node(1.0, 2), 96000).unwrap();
    let mut different_scalar_rate = ScalarTools::build(&node(1.0, 2), 96000).unwrap();
    different_rate.adopt(&hoisted);
    different_scalar_rate.adopt(&scalar);
    assert_inactive_clear(&different_rate);
    for frame in 0..192 {
        compare(
            &mut different_rate,
            &mut different_scalar_rate,
            &frames(16, frame),
            "different-rate adopt",
        );
    }
}

#[test]
fn zero_drive_identity_still_advances_fir_and_preserves_signed_zero_outputs() {
    let n = node(0.0, 1);
    let mut hoisted = VoiceTools::build(&n, 48000).unwrap();
    let mut scalar = ScalarTools::build(&n, 48000).unwrap();
    let zero = PortFrame {
        channels: 16,
        values: [-0.0; MAX_CHANNELS],
    };
    for frame in 0..131 {
        compare(
            &mut hoisted,
            &mut scalar,
            &[zero, zero],
            &format!("negative zero frame {frame}"),
        );
    }
    assert_same_state(&hoisted, &scalar);
    assert_eq!(filter(&hoisted).drive_lanes[0].head, 131 % LATENCY_SAMPLES);
    // Follow a zero-drive signal history with nonlinear drive; skipped history
    // writes would be detected after adoption by all three output ports.
    for frame in 0..137 {
        compare(
            &mut hoisted,
            &mut scalar,
            &frames(16, frame),
            "zero drive history warmup",
        );
    }
    let mut driven = VoiceTools::build(&node(1.0, 1), 48000).unwrap();
    let mut driven_scalar = ScalarTools::build(&node(1.0, 1), 48000).unwrap();
    driven.adopt(&hoisted);
    driven_scalar.adopt(&scalar);
    assert_same_state(&driven, &driven_scalar);
    for frame in 0..192 {
        compare(
            &mut driven,
            &mut driven_scalar,
            &[zero, PortFrame::default()],
            &format!("zero-to-driven history continuation frame {frame}"),
        );
    }
}

#[test]
fn drive_f64_bits_and_all_fir_cv_histories_match_original_arithmetic() {
    fn original(lane: &mut DriveLane, x: f64, cv: f64, gain: f64, blend: f64) -> (f64, f64) {
        let delayed_cv = lane.cv[lane.head];
        lane.cv[lane.head] = cv;
        lane.head = (lane.head + 1) % LATENCY_SAMPLES;
        let denom = gain.tanh();
        (
            lane.oversampler.process_pair(x, 0.0, |x, _| {
                x + blend * (5.0 * (x * gain / 5.0).tanh() / denom - x)
            }),
            delayed_cv,
        )
    }
    for drive in DRIVES {
        let drive = f64::from(drive);
        let gain = 1.0 + 8.0 * drive;
        let t = (drive / 0.1).clamp(0.0, 1.0);
        let blend = t * t * (3.0 - 2.0 * t);
        let mut hoisted = std::array::from_fn::<_, 4, _>(|_| DriveLane::new());
        for (lane, warmup) in [17, 203, 41, 89].into_iter().enumerate() {
            for frame in 0..warmup {
                original(
                    &mut hoisted[lane],
                    (frame as f64 * 0.113).sin() * 5.0,
                    lane as f64,
                    gain,
                    blend,
                );
            }
        }
        let mut scalar = hoisted.clone();
        for frame in 0..512 {
            let inputs = frames(4, frame);
            for lane in 0..4 {
                let x = if frame < 320 {
                    finite(inputs[0].values[lane]).clamp(-1e6, 1e6)
                } else {
                    0.0
                };
                let cv = if frame < 320 {
                    finite(inputs[1].values[lane])
                } else {
                    0.0
                };
                let expected = original(&mut scalar[lane], x, cv, gain, blend);
                let actual = hoisted[lane].process(x, cv, gain, blend, gain.tanh());
                assert_eq!(
                    actual.0.to_bits(),
                    expected.0.to_bits(),
                    "drive {drive}, frame {frame}, lane {lane}"
                );
                assert_eq!(actual.1.to_bits(), expected.1.to_bits());
                assert_eq!(hoisted[lane].head, scalar[lane].head);
                assert_eq!(
                    hoisted[lane].cv.map(f64::to_bits),
                    scalar[lane].cv.map(f64::to_bits)
                );
                if frame % 61 == 0 || frame == 511 {
                    assert_eq!(
                        format!("{:?}", hoisted[lane].oversampler),
                        format!("{:?}", scalar[lane].oversampler)
                    );
                }
            }
        }
    }
}

#[test]
fn signal_impulse_and_each_lane_cv_delay_are_exactly_76_host_frames() {
    assert_eq!(LATENCY_SAMPLES, 76);
    let mut impulse_lane = DriveLane::new();
    let mut impulse = [0.0_f64; 192];
    for (frame, output) in impulse.iter_mut().enumerate() {
        *output = impulse_lane
            .process(
                if frame == 0 { 1.0 } else { 0.0 },
                0.0,
                1.0,
                0.0,
                1.0_f64.tanh(),
            )
            .0;
    }
    let peak = impulse
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 76);
    let mut lanes = std::array::from_fn::<_, MAX_CHANNELS, _>(|_| DriveLane::new());
    for frame in 0..(3 * LATENCY_SAMPLES) {
        for (lane, state) in lanes.iter_mut().enumerate() {
            let cv = (lane * 1000 + frame + 1) as f64;
            let actual = state.process(0.0, cv, 1.0, 0.0, 1.0_f64.tanh()).1;
            let expected = if frame < LATENCY_SAMPLES {
                0.0
            } else {
                (lane * 1000 + frame + 1 - LATENCY_SAMPLES) as f64
            };
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "lane {lane}, frame {frame}"
            );
        }
    }
    // State-only adoption must preserve nonzero CV and its chronological head.
    let mut adopted = std::array::from_fn::<_, MAX_CHANNELS, _>(|_| DriveLane::new());
    for lane in 0..MAX_CHANNELS {
        adopted[lane].copy_state_from(&lanes[lane]);
    }
    for frame in 0..(2 * LATENCY_SAMPLES) {
        for lane in 0..MAX_CHANNELS {
            let cv = -((lane * 1000 + frame + 1) as f64);
            let expected = lanes[lane].process(0.0, cv, 1.0, 0.0, 1.0_f64.tanh());
            let actual = adopted[lane].process(0.0, cv, 9.0, 1.0, 9.0_f64.tanh());
            assert_eq!(actual.1.to_bits(), expected.1.to_bits());
        }
    }
}
