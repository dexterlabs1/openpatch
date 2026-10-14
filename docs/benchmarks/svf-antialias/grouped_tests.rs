use super::scalar_reference::VoiceTools as ScalarTools;
use super::*;

const DRIVES: [f32; 4] = [0.0, 1.0e-12, 0.1, 1.0];

fn node(drive: f32, revision: usize) -> Node {
    Node {
        score: None,
        id: "grouped-test".into(),
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
    grouped: &mut VoiceTools,
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
    grouped.process(inputs, &mut actual);
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
                let mut grouped = VoiceTools::build(&n, rate).unwrap();
                let mut scalar = ScalarTools::build(&n, rate).unwrap();
                for frame in 0..160 {
                    compare(
                        &mut grouped,
                        &mut scalar,
                        &frames(count, frame),
                        &format!("rate {rate}, drive {drive}, count {count}, frame {frame}"),
                    );
                }
                // Continue for >64 host frames after the stimulus to expose all
                // retained interpolation, decimation, CV and filter histories.
                let silence = [PortFrame {
                    channels: count as u8,
                    ..Default::default()
                }; 2];
                for frame in 0..96 {
                    compare(
                        &mut grouped,
                        &mut scalar,
                        &silence,
                        &format!("tail count {count} frame {frame}"),
                    );
                }
                assert_inactive_clear(&grouped);
            }
        }
    }
}

#[test]
fn changing_counts_keep_different_heads_and_clear_retired_lanes() {
    for drive in DRIVES {
        let n = node(drive, 1);
        let mut grouped = VoiceTools::build(&n, 48000).unwrap();
        let mut scalar = ScalarTools::build(&n, 48000).unwrap();
        let counts = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 5, 11, 2, 16, 0, 17, 3, 8,
        ];
        for (segment, count) in counts.into_iter().enumerate() {
            for frame in 0..(17 + segment % 13) {
                compare(
                    &mut grouped,
                    &mut scalar,
                    &frames(count, segment * 53 + frame),
                    &format!(
                        "drive {drive}, changing count {count}, segment {segment}, frame {frame}"
                    ),
                );
            }
            assert_inactive_clear(&grouped);
            if segment == 15 {
                // Sequential activation creates differing CV ring heads inside
                // each quartet. FIR heads also differ because they advance with
                // each lane's independently activated host-frame count.
                let f = filter(&grouped);
                assert_ne!(f.drive_lanes[0].head, f.drive_lanes[1].head);
                assert_ne!(
                    format!("{:?}", f.drive_lanes[0].oversampler),
                    format!("{:?}", f.drive_lanes[1].oversampler)
                );
            }
        }
        for frame in 0..160 {
            compare(
                &mut grouped,
                &mut scalar,
                &frames(16, 3000 + frame),
                "count-change continuation",
            );
        }
    }
}

#[test]
fn reset_and_adoption_keep_new_compiled_drive_constants_and_scalar_continuation() {
    let mut grouped = VoiceTools::build(&node(0.0, 0), 48000).unwrap();
    let mut scalar = ScalarTools::build(&node(0.0, 0), 48000).unwrap();
    for (revision, drive) in [0.0, 1.0, 1.0e-12, 0.1, 0.0, 1.0].into_iter().enumerate() {
        let n = node(drive, revision);
        let mut next_grouped = VoiceTools::build(&n, 48000).unwrap();
        let mut next_scalar = ScalarTools::build(&n, 48000).unwrap();
        let compiled = (
            filter(&next_grouped).gain,
            filter(&next_grouped).blend,
            filter(&next_grouped).drive_denom,
        );
        next_grouped.adopt(&grouped);
        next_scalar.adopt(&scalar);
        assert_eq!(
            compiled,
            (
                filter(&next_grouped).gain,
                filter(&next_grouped).blend,
                filter(&next_grouped).drive_denom
            )
        );
        grouped = next_grouped;
        scalar = next_scalar;
        for frame in 0..240 {
            compare(
                &mut grouped,
                &mut scalar,
                &frames([16, 3, 13, 8][(frame / 53) % 4], revision * 241 + frame),
                &format!("adopt revision {revision}, drive {drive}, frame {frame}"),
            );
            if frame == 119 {
                grouped.reset_state();
                scalar.reset_state();
                assert_inactive_clear(&grouped);
            }
        }
    }
    // The scalar prototype does not adopt across sample-rate changes.
    let mut different_rate = VoiceTools::build(&node(1.0, 2), 96000).unwrap();
    let mut different_scalar_rate = ScalarTools::build(&node(1.0, 2), 96000).unwrap();
    different_rate.adopt(&grouped);
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
    let mut grouped = VoiceTools::build(&n, 48000).unwrap();
    let mut scalar = ScalarTools::build(&n, 48000).unwrap();
    let zero = PortFrame {
        channels: 16,
        values: [-0.0; MAX_CHANNELS],
    };
    for frame in 0..131 {
        compare(
            &mut grouped,
            &mut scalar,
            &[zero, zero],
            &format!("negative zero frame {frame}"),
        );
    }
    assert_eq!(filter(&grouped).drive_lanes[0].head, 131 % LATENCY_SAMPLES);
    // Follow a zero-drive signal history with nonlinear drive; skipped history
    // writes would be detected after adoption by all three output ports.
    for frame in 0..137 {
        compare(
            &mut grouped,
            &mut scalar,
            &frames(16, frame),
            "zero drive history warmup",
        );
    }
    let mut driven = VoiceTools::build(&node(1.0, 1), 48000).unwrap();
    let mut driven_scalar = ScalarTools::build(&node(1.0, 1), 48000).unwrap();
    driven.adopt(&grouped);
    driven_scalar.adopt(&scalar);
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
fn grouped_drive_f64_bits_and_fir_histories_match_original_with_different_ring_heads() {
    // Keep the original scalar expression here to check the resampler before
    // f32 output conversion and before any SVF state can conceal a discrepancy.
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
        let mut grouped = std::array::from_fn::<_, 4, _>(|_| DriveLane::new());
        for (lane, warmup) in [17, 203, 41, 89].into_iter().enumerate() {
            for frame in 0..warmup {
                original(
                    &mut grouped[lane],
                    (frame as f64 * 0.113).sin() * 5.0,
                    lane as f64,
                    gain,
                    blend,
                );
            }
        }
        let mut scalar = grouped.clone();
        for frame in 0..300 {
            let inputs = frames(4, frame);
            let x = std::array::from_fn(|lane| finite(inputs[0].values[lane]).clamp(-1e6, 1e6));
            let cv = std::array::from_fn(|lane| finite(inputs[1].values[lane]));
            let expected = std::array::from_fn::<_, 4, _>(|lane| {
                original(&mut scalar[lane], x[lane], cv[lane], gain, blend)
            });
            let [first, second, third, fourth] = &mut grouped;
            let (actual, delayed_cv) = DriveLane::process_four(
                first,
                second,
                third,
                fourth,
                x,
                cv,
                gain,
                blend,
                gain.tanh(),
            );
            for lane in 0..4 {
                assert_eq!(
                    actual[lane].to_bits(),
                    expected[lane].0.to_bits(),
                    "f64 drive {drive}, frame {frame}, lane {lane}"
                );
                assert_eq!(delayed_cv[lane].to_bits(), expected[lane].1.to_bits());
                assert_eq!(grouped[lane].head, scalar[lane].head);
                assert_eq!(
                    grouped[lane].cv.map(f64::to_bits),
                    scalar[lane].cv.map(f64::to_bits)
                );
                if frame % 61 == 0 {
                    // Debug f64 formatting round-trips values and preserves -0;
                    // this checks every FIR slot, ring head and control counter.
                    assert_eq!(
                        format!("{:?}", grouped[lane].oversampler),
                        format!("{:?}", scalar[lane].oversampler)
                    );
                }
            }
        }
        // Switch each grouped lane to scalar processing and flush more than the
        // entire FIR latency: hidden differences must survive this comparison.
        for frame in 0..128 {
            for lane in 0..4 {
                let expected = original(&mut scalar[lane], 0.0, 0.0, gain, blend);
                let actual = grouped[lane].process(0.0, 0.0, gain, blend, gain.tanh());
                assert_eq!(
                    actual.0.to_bits(),
                    expected.0.to_bits(),
                    "continuation drive {drive}, frame {frame}, lane {lane}"
                );
                assert_eq!(actual.1.to_bits(), expected.1.to_bits());
            }
        }
    }
}

#[test]
fn each_cv_lane_has_exact_64_host_frame_delay() {
    let mut lanes = std::array::from_fn::<_, 4, _>(|_| DriveLane::new());
    for (lane, warmup) in [17, 203, 41, 89].into_iter().enumerate() {
        for frame in 0..warmup {
            lanes[lane].process(
                0.0,
                -((lane * 1000 + frame + 1) as f64),
                1.0,
                0.0,
                1.0_f64.tanh(),
            );
        }
    }
    let mut expected_cv = lanes.clone();
    for frame in 0..193 {
        let cv = std::array::from_fn(|lane| (lane * 1000 + frame + 1) as f64);
        let expected = std::array::from_fn::<_, 4, _>(|lane| expected_cv[lane].delay_cv(cv[lane]));
        let [first, second, third, fourth] = &mut lanes;
        let (_, delayed) = DriveLane::process_four(
            first,
            second,
            third,
            fourth,
            [0.0; 4],
            cv,
            1.0,
            0.0,
            1.0_f64.tanh(),
        );
        assert_eq!(delayed.map(f64::to_bits), expected.map(f64::to_bits));
        if frame >= LATENCY_SAMPLES {
            assert_eq!(
                delayed,
                std::array::from_fn(|lane| (lane * 1000 + frame + 1 - LATENCY_SAMPLES) as f64)
            );
        }
    }
}
