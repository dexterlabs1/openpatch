use super::*;

fn node(drive: f32, cutoff: f32, resonance: f32, cv_amount: f32) -> Node {
    Node {
        content: None,
        score: None,
        id: "svf-qualification".into(),
        kind: "svf_filter".into(),
        bypassed: false,
        params: [
            ("drive", drive),
            ("cutoff", cutoff),
            ("resonance", resonance),
            ("cutoff_cv_amount", cv_amount),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect(),
    }
}

fn filter(tool: &VoiceTools) -> &Filter {
    let VoiceTools::Filter(filter) = tool else {
        panic!("expected filter")
    };
    filter
}

fn tick(tool: &mut VoiceTools, signal: PortFrame, cv: PortFrame) -> [PortFrame; 3] {
    let mut outputs = [PortFrame::default(); 3];
    tool.process(&[signal, cv], &mut outputs);
    outputs
}

/// Independent implicit trapezoidal solution of the continuous two-state ODE:
/// db/dt = omega*(x-k*b-l), dl/dt = omega*b. Frequency prewarping sets
/// g = tan(pi*cutoff/rate), the normalized half-step. Keep physical output
/// states and the previous half-step derivatives rather than TPT integrator
/// memories. Solve the resulting 2x2 system by Gaussian elimination.
#[derive(Clone, Copy, Default)]
struct TrapezoidalReference {
    band: f64,
    low: f64,
    previous_band_step: f64,
    previous_low_step: f64,
}

impl TrapezoidalReference {
    fn step(&mut self, x: f64, cutoff: f64, resonance: f64, rate: f64) -> [f64; 3] {
        let half_step = (PI * cutoff / rate).tan();
        let damping = 2.0 - 1.95 * resonance;
        let rhs_band = self.band + self.previous_band_step + half_step * x;
        let rhs_low = self.low + self.previous_low_step;
        let diagonal = 1.0 + half_step * damping;
        let normalized_rhs = rhs_band / diagonal;
        let normalized_upper = half_step / diagonal;
        let low = (rhs_low + half_step * normalized_rhs) / (1.0 + half_step * normalized_upper);
        let band = normalized_rhs - normalized_upper * low;
        let high = x - damping * band - low;
        self.previous_band_step = half_step * high;
        self.previous_low_step = half_step * band;
        self.band = band;
        self.low = low;
        [low, band, high]
    }
}

fn cutoff_controls(channels: u8, frame: usize) -> PortFrame {
    let mut cv = PortFrame {
        channels,
        ..Default::default()
    };
    for lane in 0..usize::from(channels) {
        cv.values[lane] = [-1.25, 0.75, -0.5, 1.25, 0.0][(frame / 19 + lane * 2) % 5];
    }
    cv
}

#[test]
fn modulated_cutoff_matches_independent_trapezoidal_outputs_after_76_sample_delay() {
    // Warm the signal FIR to DC, then initialize only the SVF integrators to
    // zero. This produces an observable filter transient with a known constant
    // input, without reusing the candidate signal FIR in the reference. A fully
    // settled DC filter would not reveal cutoff-CV timing errors.
    const DELAY: usize = 76;
    const TOLERANCE_V: f64 = 3.0e-6;
    for rate in [44100, 48000, 96000] {
        for cv_channels in [1, 3, 16] {
            let mut tool = VoiceTools::build(&node(0.0, 310.0, 0.97, 1.25), rate).unwrap();
            let signal = PortFrame {
                channels: 16,
                values: std::array::from_fn(|lane| {
                    (0.1 + lane as f32 * 0.075) * if lane % 2 == 0 { 1.0 } else { -1.0 }
                }),
            };
            for _ in 0..400 {
                tick(&mut tool, signal, PortFrame::default());
            }
            let VoiceTools::Filter(f) = &mut tool else {
                unreachable!()
            };
            f.states = [[0.0; 2]; MAX_CHANNELS];
            let resonance = f64::from(0.97_f32);
            let mut reference = [TrapezoidalReference::default(); MAX_CHANNELS];
            let mut early_reference = reference;
            let mut late_reference = reference;
            let mut early_error = 0.0_f64;
            let mut late_error = 0.0_f64;
            for frame in 0..512 {
                let outputs = tick(&mut tool, signal, cutoff_controls(cv_channels, frame));
                for lane in 0..MAX_CHANNELS {
                    let delayed_control = |delay| {
                        if frame < delay {
                            0.0
                        } else {
                            f64::from(cutoff_controls(cv_channels, frame - delay).voice(lane, 0.0))
                        }
                    };
                    let cutoff = |cv: f64| {
                        (310.0 * 2.0_f64.powf(cv * 1.25)).clamp(1.0, f64::from(rate) * 0.45)
                    };
                    let x = f64::from(signal.values[lane]);
                    let expected = reference[lane].step(
                        x,
                        cutoff(delayed_control(DELAY)),
                        resonance,
                        rate.into(),
                    );
                    let early = early_reference[lane].step(
                        x,
                        cutoff(delayed_control(DELAY - 1)),
                        resonance,
                        rate.into(),
                    );
                    let late = late_reference[lane].step(
                        x,
                        cutoff(delayed_control(DELAY + 1)),
                        resonance,
                        rate.into(),
                    );
                    for port in 0..3 {
                        let actual = f64::from(outputs[port].values[lane]);
                        let tolerance = TOLERANCE_V * x.abs().max(1.0);
                        assert!((actual - expected[port]).abs() <= tolerance,
                            "rate {rate}, CV channels {cv_channels}, frame {frame}, lane {lane}, port {port}: actual {actual}, reference {}, tolerance {tolerance}", expected[port]);
                        early_error = early_error.max((actual - early[port]).abs());
                        late_error = late_error.max((actual - late[port]).abs());
                    }
                }
            }
            // Verify this fixture can reject a one-sample error on either side.
            assert!(
                early_error > 1.0e-4,
                "75-sample reference was insufficiently distinguishable"
            );
            assert!(
                late_error > 1.0e-4,
                "77-sample reference was insufficiently distinguishable"
            );
        }
    }
}

#[test]
fn drive_cutoff_resonance_adoption_preserves_histories_and_recovers_to_silence() {
    const OUTPUT_BOUND_V: f64 = 256.0;
    const SILENCE_BOUND_V: f64 = 1.0e-7;
    for rate in [44100, 48000, 96000] {
        let mut tool = VoiceTools::build(&node(0.0, 1000.0, 0.2, 0.5), rate).unwrap();
        let mut frame = 0;
        let mut observed_peak = 0.0_f64;
        for (drive, cutoff, resonance) in [
            (0.0, 5.0, 1.0),
            (f32::EPSILON, 20000.0, 0.0),
            (0.1, 731.0, 0.95),
            (1.0, 4800.0, 1.0),
        ] {
            let mut next = VoiceTools::build(&node(drive, cutoff, resonance, 0.5), rate).unwrap();
            let compiled = (
                filter(&next).gain,
                filter(&next).blend,
                filter(&next).drive_denom,
            );
            next.adopt(&tool);
            assert_eq!(
                filter(&next).states.map(|lane| lane.map(f64::to_bits)),
                filter(&tool).states.map(|lane| lane.map(f64::to_bits))
            );
            assert_eq!(filter(&next).active, filter(&tool).active);
            for lane in 0..MAX_CHANNELS {
                assert_eq!(
                    format!("{:?}", filter(&next).drive_lanes[lane]),
                    format!("{:?}", filter(&tool).drive_lanes[lane]),
                    "rate {rate}, drive {drive}, adopted lane {lane}"
                );
            }
            assert_eq!(
                compiled,
                (
                    filter(&next).gain,
                    filter(&next).blend,
                    filter(&next).drive_denom
                )
            );
            assert_eq!(filter(&next).cutoff, f64::from(cutoff));
            assert_eq!(filter(&next).resonance, f64::from(resonance));
            tool = next;
            for _ in 0..384 {
                let signal = PortFrame {
                    channels: 16,
                    values: std::array::from_fn(|lane| {
                        let phase =
                            TAU * 733.0 * frame as f64 / f64::from(rate) + lane as f64 * 0.17;
                        (5.0 * phase.sin()) as f32
                    }),
                };
                let cv = PortFrame {
                    channels: 16,
                    values: std::array::from_fn(|lane| {
                        (0.25 * (frame as f64 * 0.037 + lane as f64 * 0.11).sin()) as f32
                    }),
                };
                let outputs = tick(&mut tool, signal, cv);
                for out in outputs {
                    assert_eq!(out.channels, 16);
                    for value in out.values {
                        let magnitude = f64::from(value).abs();
                        assert!(
                            value.is_finite() && magnitude < OUTPUT_BOUND_V,
                            "rate {rate}, drive {drive}, frame {frame}: output {value}"
                        );
                        observed_peak = observed_peak.max(magnitude);
                    }
                }
                frame += 1;
            }
        }
        assert!(observed_peak > 0.1, "fixture did not excite the filter");
        let silence = PortFrame {
            channels: 16,
            ..Default::default()
        };
        let mut final_peak = 0.0_f64;
        for frame in 0..4096 {
            let outputs = tick(&mut tool, silence, PortFrame::default());
            for out in outputs {
                for value in out.values {
                    assert!(value.is_finite() && f64::from(value).abs() < OUTPUT_BOUND_V);
                    if frame >= 3968 {
                        final_peak = final_peak.max(f64::from(value).abs());
                    }
                }
            }
        }
        assert!(
            final_peak < SILENCE_BOUND_V,
            "rate {rate}: last 128 silent frames peaked at {final_peak} V"
        );
        assert!(filter(&tool)
            .states
            .iter()
            .flatten()
            .all(|state| state.is_finite() && state.abs() < SILENCE_BOUND_V));
    }
}
