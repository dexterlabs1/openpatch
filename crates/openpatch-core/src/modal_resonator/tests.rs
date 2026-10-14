use super::*;
use serde_json::json;
fn node(params: Value) -> Node {
    serde_json::from_value(json!({"id":"body","kind":"modal_resonator","params":params})).unwrap()
}
fn resonator(params: Value, rate: u32) -> ModalResonator {
    ModalResonator::build(&node(params), rate).unwrap()
}
fn step(
    body: &mut ModalResonator,
    pitch: PortFrame,
    strike: PortFrame,
    force: PortFrame,
) -> PortFrame {
    let mut out = [PortFrame::default()];
    body.process(&[pitch, strike, force], &mut out);
    out[0]
}
fn silent_step(body: &mut ModalResonator) -> PortFrame {
    step(
        body,
        PortFrame::default(),
        PortFrame::default(),
        PortFrame::default(),
    )
}
fn close(a: f64, b: f64, tolerance: f64) {
    assert!(
        (a - b).abs() <= tolerance,
        "{a} != {b}, error {}",
        (a - b).abs()
    );
}

#[test]
fn exact_impulse_matches_independent_analytic_solution_across_rates() {
    // Each mode has a closed-form initial-value solution, independent of the
    // production coefficient builder or mode metadata helper.
    for rate in [8000, 44100, 48000, 96000, 192000] {
        for (f, decay, damping, b) in [
            (20.0, 0.1, 1.0, 0.0),
            (220.0, 2.0, 0.3, 0.2),
            (999.0, 30.0, 0.0, 1.0),
        ] {
            let mut body = resonator(
                json!({"frequency":f,"decay":decay,"damping":damping,"stiffness":b,"strike_position":0.25,"pickup_position":0.5}),
                rate,
            );
            for frame in 0..2048 {
                let actual = step(
                    &mut body,
                    PortFrame::default(),
                    PortFrame::mono(if frame == 0 { 10.0 } else { 0.0 }),
                    PortFrame::default(),
                )
                .values[0];
                let time = (frame + 1) as f64 / f64::from(rate);
                let mut expected = 0.0;
                for n in 1..=16 {
                    let n = n as f64;
                    let b = f64::from(b as f32);
                    let frequency = f * ((1.0 - b) * n * n + b * n.powi(4)).sqrt();
                    if frequency > f64::from(rate) * 0.45 {
                        continue;
                    }
                    // Params arrive as f32, so use their actual encoded damping.
                    let loss = std::f64::consts::LN_10 * 3.0 / f64::from(decay as f32)
                        * (1.0 + f64::from(damping as f32) * (n - 1.0));
                    let ringing = ((TAU * frequency).powi(2) - loss * loss).sqrt();
                    let v0 = 0.02 * (n * PI * 0.25).sin();
                    expected += 25.0
                        * (n * PI * 0.5).sin()
                        * v0
                        * (-loss * time).exp()
                        * ((ringing * time).cos() - loss / ringing * (ringing * time).sin());
                }
                close(f64::from(actual), expected, 2e-7);
            }
        }
    }
}

#[test]
fn held_force_matches_closed_form_step_and_static_deflection() {
    for rate in [8000, 48000, 192000] {
        let mut body = resonator(
            json!({"frequency":20,"decay":0.1,"damping":1,"stiffness":0,"strike_strength":0,"strike_position":0.5}),
            rate,
        );
        for k in 1..=rate / 2 {
            step(
                &mut body,
                PortFrame::default(),
                PortFrame::default(),
                PortFrame::mono(3.0),
            );
            if [1, 17, rate / 2].contains(&k) {
                for (i, state) in body.voices[0].states.iter().enumerate() {
                    let n = (i + 1) as f64;
                    let f = 20.0 * n;
                    if f > rate as f64 * 0.45 {
                        continue;
                    }
                    let w2 = (TAU * f).powi(2);
                    let s = LN_1000 / f64::from(0.1_f32) * n;
                    let d = (w2 - s * s).sqrt();
                    let t = k as f64 / rate as f64;
                    let u = 6.0 * (n * PI * 0.5).sin();
                    let r = (-s * t).exp();
                    let expected_q = u / w2 * (1.0 - r * ((d * t).cos() + s / d * (d * t).sin()));
                    let expected_v = u * r * (d * t).sin() / d;
                    close(state.q, expected_q, 1e-12);
                    close(state.v, expected_v, 1e-11);
                }
            }
        }
        close(
            body.voices[0].states[0].q,
            6.0 / (TAU * 20.0).powi(2),
            1e-12,
        );
    }
}

#[test]
fn unforced_mechanical_energy_decreases_and_rt60_frequency_are_correct() {
    let mut body = resonator(
        json!({"frequency":100,"decay":1,"damping":0,"stiffness":0}),
        48000,
    );
    silent_step(&mut body);
    body.voices[0].states[0] = ModeState { q: 0.001, v: 0.0 };
    let w = TAU * 100.0;
    let mut energy = 0.5 * MASS * w * w * 0.001_f64.powi(2);
    let mut positive_crossings = Vec::new();
    let mut previous = 0.001;
    for frame in 1..=48000 {
        silent_step(&mut body);
        let state = body.voices[0].states[0];
        let new_energy = 0.5 * MASS * (state.v * state.v + w * w * state.q * state.q);
        assert!(new_energy <= energy * (1.0 + 2e-13));
        energy = new_energy;
        if previous < 0.0 && state.q >= 0.0 {
            positive_crossings.push(frame);
        }
        previous = state.q;
    }
    let s = LN_1000;
    let d = (w * w - s * s).sqrt();
    let expected_q = 0.001 * (-s).exp() * (d.cos() + s / d * d.sin());
    close(body.voices[0].states[0].q, expected_q, 1e-15);
    let period = positive_crossings
        .windows(2)
        .map(|w| (w[1] - w[0]) as f64)
        .sum::<f64>()
        / (positive_crossings.len() - 1) as f64;
    close(48000.0 / period, d / TAU, 0.01);
    // Measure the actual state envelope, independent of zero crossings.
    let state = body.voices[0].states[0];
    let envelope = (state.q.powi(2) + ((state.v + s * state.q) / d).powi(2)).sqrt();
    let initial_envelope = (0.001_f64.powi(2) + (s * 0.001 / d).powi(2)).sqrt();
    close(envelope / initial_envelope, 0.001, 2e-14);
}

#[test]
fn exact_zoh_converges_at_identical_physical_times_without_input_gain_scaling() {
    let mut responses = Vec::new();
    for rate in [8000, 16000, 48000, 192000] {
        let mut body = resonator(
            json!({"frequency":100,"stiffness":0,"strike_position":0.5,"pickup_position":0.5}),
            rate,
        );
        let mut last = PortFrame::default();
        for _ in 0..rate / 100 {
            last = step(
                &mut body,
                PortFrame::default(),
                PortFrame::default(),
                PortFrame::mono(1.0),
            );
        }
        responses.push(last.values[0]);
    }
    for value in &responses[1..] {
        close(f64::from(*value), f64::from(responses[0]), 1e-7);
    }
}

#[test]
fn trigger_hysteresis_and_polyphony_retirement_reset_physical_state() {
    let mut body = resonator(json!({}), 48000);
    let sixteen = PortFrame {
        channels: 16,
        ..PortFrame::default()
    };
    step(
        &mut body,
        sixteen,
        PortFrame::mono(10.0),
        PortFrame::default(),
    );
    assert_eq!(body.active, 16);
    let mut reference = body.clone();
    for _ in 0..50 {
        let a = step(
            &mut body,
            sixteen,
            PortFrame::mono(0.5),
            PortFrame::default(),
        );
        let b = step(
            &mut reference,
            sixteen,
            PortFrame::mono(10.0),
            PortFrame::default(),
        );
        assert_eq!(a, b);
    }
    silent_step(&mut body);
    assert_eq!(body.active, 1);
    for voice in &body.voices[1..] {
        assert!(voice.states.iter().all(|s| s.q == 0.0 && s.v == 0.0));
        assert!(!voice.high);
    }
    let frame = step(
        &mut body,
        sixteen,
        PortFrame::mono(10.0),
        PortFrame::default(),
    );
    assert!(frame.values[1..].iter().all(|v| *v == frame.values[1]));
    assert!(frame.values[1].abs() > 0.001);
    // A short poly trigger does not broadcast to extra lanes.
    let mut short = resonator(json!({}), 48000);
    let mut strike = PortFrame {
        channels: 2,
        ..PortFrame::default()
    };
    strike.values[0] = 10.0;
    let out = step(&mut short, sixteen, strike, PortFrame::default());
    assert!(out.values[0].abs() > 0.001);
    assert!(out.values[1..].iter().all(|v| *v == 0.0));
}

#[test]
fn coefficients_omit_high_modes_and_parameter_adoption_keeps_physical_state() {
    let mut old = resonator(json!({"frequency":20,"stiffness":0}), 48000);
    step(
        &mut old,
        PortFrame::default(),
        PortFrame::mono(10.0),
        PortFrame::default(),
    );
    let mut new = resonator(json!({"frequency":2000,"stiffness":1,"decay":4}), 48000);
    new.adopt(&old, true);
    for (a, b) in new.voices[0].states.iter().zip(&old.voices[0].states) {
        close(a.q, b.q, 0.0);
        close(a.v, b.v, 0.0);
    }
    assert!(new.voices[0].high);
    silent_step(&mut new);
    for (i, c) in new.voices[0].coefficients.iter().enumerate() {
        if (i + 1) > 3 {
            assert!(!c.active);
            assert_eq!(new.voices[0].states[i].q, 0.0);
            assert_eq!(new.voices[0].states[i].v, 0.0);
        }
    }
    let mut changed = old.clone();
    changed.adopt(&old, false);
    assert!(!changed.voices[0].high);
    let mut different_rate = resonator(json!({}), 96000);
    different_rate.adopt(&old, true);
    assert_eq!(different_rate.active, 0);
}

#[test]
fn spatial_nodes_suppress_even_modes_without_renormalization_and_inputs_are_safe() {
    let mut body = resonator(
        json!({"stiffness":0,"strike_position":0.5,"pickup_position":0.5}),
        48000,
    );
    step(
        &mut body,
        PortFrame::mono(f32::INFINITY),
        PortFrame::mono(10.0),
        PortFrame::mono(f32::NAN),
    );
    for i in (1..16).step_by(2) {
        assert!(body.voices[0].states[i].v.abs() < 1e-15);
    }
    for force in [f32::MAX, -f32::MAX, f32::NAN, f32::INFINITY] {
        let out = step(
            &mut body,
            PortFrame::mono(-f32::MAX),
            PortFrame::mono(force),
            PortFrame::mono(force),
        );
        assert!(out
            .values
            .iter()
            .all(|v| v.is_finite() && v.abs() <= SIGNAL_LIMIT));
    }
    body.voices[0].states[0].v = 1e200;
    silent_step(&mut body);
    assert_eq!(body.voices[0].states[0].v, 0.0);
}

#[test]
fn shared_mode_metadata_reports_physical_spectrum_loss_and_reciprocity() {
    for stiffness in [0.0, 1.0] {
        let n = node(
            json!({"frequency":100,"stiffness":stiffness,"decay":3,"damping":0.5,"strike_position":0.25,"pickup_position":0.75}),
        );
        let mut swapped = n.clone();
        swapped.params.insert("strike_position".into(), 0.75);
        swapped.params.insert("pickup_position".into(), 0.25);
        let modes = modal_resonator_modes(&n, 48000).unwrap();
        let reciprocal = modal_resonator_modes(&swapped, 48000).unwrap();
        for (i, (mode, swap)) in modes.iter().zip(&reciprocal).enumerate() {
            let index = (i + 1) as f64;
            close(
                mode.frequency_hz,
                100.0 * index.powf(1.0 + stiffness),
                1e-10,
            );
            close(mode.decay_seconds, 3.0 / (1.0 + 0.5 * (index - 1.0)), 1e-15);
            close(mode.coupling, swap.coupling, 0.0);
            assert!(mode.damped_frequency_hz < mode.frequency_hz);
        }
    }
}
