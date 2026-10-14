use super::*;
use std::f64::consts::{PI, TAU};

// Independent equation-based kernel generation and direct, high-rate FIR.
// No production coefficients, polyphase decomposition, symmetry or ring logic
// is reused by this deliberately slow reference.
fn direct_kernel(n: usize, rate: f64, cutoff: f64, halfband: bool) -> Vec<f64> {
    let mut h = (0..n).map(|i| {
        let t = i as f64 - (n - 1) as f64 / 2.0;
        let w = 0.42 - 0.5 * (TAU * i as f64 / (n - 1) as f64).cos()
            + 0.08 * (2.0 * TAU * i as f64 / (n - 1) as f64).cos();
        if halfband && t != 0.0 && t as i64 % 2 == 0 { return 0.0; }
        let sinc = if t == 0.0 { 2.0 * cutoff / rate }
            else { (TAU * cutoff * t / rate).sin() / (PI * t) };
        w * sinc
    }).collect::<Vec<_>>();
    let sum = h.iter().sum::<f64>();
    for x in &mut h { *x /= sum; }
    h
}
struct DirectFir { h: Vec<f64>, history: Vec<f64> }
impl DirectFir {
    fn new(h: Vec<f64>) -> Self { Self { history: vec![0.0; h.len()], h } }
    fn tick(&mut self, x: f64) -> f64 {
        self.history.rotate_right(1);
        self.history[0] = x;
        self.h.iter().zip(&self.history).map(|(h,x)| h*x).sum()
    }
}
struct Direct {
    up: [DirectFir; 4],
    down: [DirectFir; 4],
}
impl Direct {
    fn new() -> Self {
        let specs = [(129,2.0,0.45,false), (33,4.0,1.0,true),
            (17,8.0,2.0,true), (33,16.0,4.0,true)];
        Self {
            up: specs.map(|(n,r,c,b)| DirectFir::new(direct_kernel(n,r,c,b))),
            down: specs.map(|(n,r,c,b)| DirectFir::new(direct_kernel(n,r,c,b))),
        }
    }
    fn process(&mut self, x: f64, mut nonlinear: impl FnMut(f64) -> f64) -> f64 {
        let mut samples = vec![x];
        for filter in &mut self.up {
            samples = samples.into_iter().flat_map(|x| {
                let a = 2.0 * filter.tick(x);
                let b = 2.0 * filter.tick(0.0);
                [a,b]
            }).collect();
        }
        for x in &mut samples { *x = nonlinear(*x); }
        for filter in self.down.iter_mut().rev() {
            let filtered = samples.into_iter().map(|x| filter.tick(x)).collect::<Vec<_>>();
            samples = filtered.into_iter().step_by(2).collect();
        }
        samples[0]
    }
}

#[test]
fn optimized_matches_independent_direct_fir_and_nonlinear_reference() {
    let mut actual = Oversampler::new();
    let mut direct = Direct::new();
    let mut random = 0x1177445522996633_u64;
    let mut phase_actual = 0_u64;
    let mut phase_direct = 0_u64;
    for n in 0..900 {
        random ^= random << 13; random ^= random >> 7; random ^= random << 17;
        let input = if n < 120 { if n == 0 { 1.0 } else { 0.0 } }
            else { (random as i64 as f64 / i64::MAX as f64) * 12.0 };
        if n == 400 {
            let mut replacement = Oversampler::new();
            replacement.copy_state_from(&actual);
            actual = replacement;
        }
        let a = actual.process_pair(input, 0.0, |x,cv| {
            assert_eq!(cv, 0.0);
            let scale = 1.0 + 0.125 * (phase_actual as f64 * 0.011).sin();
            phase_actual += 1;
            x + 0.7 * (5.0 * (x * scale / 5.0).tanh() / scale.tanh() - x)
        });
        let b = direct.process(input, |x| {
            let scale = 1.0 + 0.125 * (phase_direct as f64 * 0.011).sin();
            phase_direct += 1;
            x + 0.7 * (5.0 * (x * scale / 5.0).tanh() / scale.tanh() - x)
        });
        assert!((a-b).abs() < 2e-12, "sample {n}: {a} != {b}");
    }
    assert_eq!(phase_actual, 900*16);
    assert_eq!(phase_actual, phase_direct);
}

#[test]
fn phase_zero_impulse_delay_and_linear_dc() {
    let mut os = Oversampler::new();
    let impulse = (0..300).map(|n| os.process_pair(f64::from(n==0), 0.0, |x,_|x))
        .collect::<Vec<_>>();
    let peak = impulse.iter().enumerate().max_by(|a,b| a.1.abs().total_cmp(&b.1.abs())).unwrap().0;
    assert_eq!(peak, LATENCY_SAMPLES);
    for k in 1..LATENCY_SAMPLES {
        assert!((impulse[LATENCY_SAMPLES-k] - impulse[LATENCY_SAMPLES+k]).abs() < 1e-13);
    }
    assert!((impulse.iter().sum::<f64>()-1.0).abs() < 1e-6);
    let mut os = Oversampler::new();
    for n in 0..500 {
        let value = os.process_pair(1.0, 0.0, |x,_|x);
        if n > 200 { assert!((value-1.0).abs() < 1e-6, "DC {value}"); }
    }
}

#[test]
fn linear_gain_and_phase_through_18khz_at_44100() {
    for frequency in [1000.0, 10000.0, 18000.0] {
        let mut os = Oversampler::new();
        let omega = TAU * frequency / 44100.0;
        let mut in_phase = 0.0;
        let mut quadrature = 0.0;
        let mut expected_energy = 0.0;
        for n in 0..4500 {
            let value = os.process_pair((omega*n as f64).sin(), 0.0, |x,_|x);
            if n > 300 {
                let phase = omega*(n as f64-LATENCY_SAMPLES as f64);
                in_phase += value * phase.sin();
                quadrature += (value-phase.sin()) * phase.cos();
                expected_energy += phase.sin().powi(2);
            }
        }
        let gain = in_phase / expected_energy;
        assert!(20.0*gain.log10() > -0.025, "{frequency}: {gain}");
        assert!((quadrature / expected_energy).abs() < 5e-6, "{frequency}: phase {quadrature}");
    }
}
