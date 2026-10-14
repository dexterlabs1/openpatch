//! Original seven-output noise synthesis. Filter coefficients are generated on
//! the control thread from mathematical transfer functions, never from plugin
//! source. All callback histories and coefficients have fixed capacity.
use crate::{parameter, Node, PortFrame};
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};
use std::sync::{Mutex, OnceLock};

#[path = "colored_noise_gray.rs"]
mod gray;
pub(crate) const OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];
const SECTIONS: usize = 16;
const NOMINAL_RMS: f64 = 5.0 / std::f64::consts::SQRT_2;

pub(crate) fn catalog() -> Value {
    json!({"kind":"colored_noise","title":"NOISE","model":"functional","inputs":[],"params":{
        "seed":{"default":1.0,"min":0.0,"max":16777215.0,"step":1.0,"description":"24-bit deterministic seed. Changes restart every stream and filter history."},
        "level":{"default":1.0,"min":0.0,"max":2.0,"unit":"x","description":"Shared output multiplier; six Gaussian colors have nominal 5/sqrt(2) V RMS at 1; black is uniform within ±5 V."}},
        "description":"Seven simultaneous mono noise colors: Gaussian white, pink (−3 dB/oct), red (−6), violet (+6), blue (+3), inverse A-weighted gray, and uniform black. Original bounded IIR synthesis; no peak limiter on Gaussian outputs. Nominal stationary variance is normalized per sample rate."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"] =
        json!("seven simultaneous mono generators; no inputs or voice-count parameter");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["functional_reference"] = json!("https://vcvrack.com/Free#vcv-noise");
    module["compatibility"] = json!("original MIT Rust implementation of seven noise-color roles and voltage conventions; deterministic seed, extra level control, IIR edge behavior and shared-stream correlations are explicit OpenPatch policies, not sample-identical Rack DSP");
    module["state_adoption"] = json!("same ID/kind/sample rate/seed copies PRNG state, Gaussian spare and every filter history; new level applies immediately; changed seed or sample rate starts fresh; wiring and selected output do not affect stream advancement; saved patches store parameters, not transient noise state");
    module["initialize_policy"] = json!("restart all PRNG streams and filter histories using compiled seed; parameter Initialize also restores seed=1 and level=1");
    module["noise_policy"] = json!({"rms_volts_at_level_one":NOMINAL_RMS,"black":{"distribution":"uniform","minimum_volts":-5.0,"maximum_volts":5.0,"rms_volts":5.0/3.0_f64.sqrt()},"gaussian_peaks":"unbounded distribution; no clipping or peak limiter","streams":"white drives red/violet/gray; independent Gaussian pink drives blue; black has a separate uniform stream","startup":"zero filter histories; stationary RMS specification excludes initial settling","spectral_band_hz":{"minimum":100,"maximum":"min(10000, sample_rate/8)"},"pink":"16 logarithmically interlaced matched-pole/zero shelves, lowest pole 1 Hz","red":"matched leaky integrator, 5 Hz low-frequency plateau","violet":"first difference of white, with discrete-time high-frequency curvature","blue":"first difference of unnormalized pink, with discrete-time high-frequency curvature","gray":"inverse A weighting approximated by 12 digital peaking sections; fourth-order high-pass at 60 Hz and fourth-order low-pass at min(22000 Hz, 0.46 * sample_rate); qualified shape 80..min(20000 Hz, 0.4 * sample_rate), soft edge rolloffs","coefficient_cache":"control-thread-only, bounded 16 sample-rate entries; callback performs no allocation, locks or coefficient generation"});
    for (index, output) in module["output_metadata"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        output["maximum_channels"] = json!(1);
        output["nominal_rms_volts"] = json!(if index == 6 {
            5.0 / 3.0_f64.sqrt()
        } else {
            NOMINAL_RMS
        });
        output["psd_slope_db_per_octave"] = json!(match index {
            0 | 6 => Some(0.0),
            1 => Some(-3.0),
            2 => Some(-6.0),
            3 => Some(6.0),
            4 => Some(3.0),
            _ => None,
        });
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Biquad {
    pub(crate) b: [f64; 3],
    pub(crate) a: [f64; 2],
}
impl Biquad {
    fn power(self, omega: f64) -> f64 {
        // Real trigonometric expansion avoids complex arithmetic/allocation.
        let polynomial = |c: [f64; 3]| {
            let real = c[0] + c[1] * omega.cos() + c[2] * (2.0 * omega).cos();
            let imaginary = c[1] * omega.sin() + c[2] * (2.0 * omega).sin();
            real * real + imaginary * imaginary
        };
        polynomial(self.b) / polynomial([1.0, self.a[0], self.a[1]])
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct History {
    z1: f64,
    z2: f64,
}
impl History {
    #[inline]
    fn process(&mut self, c: Biquad, x: f64) -> f64 {
        let y = c.b[0] * x + self.z1;
        self.z1 = c.b[1] * x - c.a[0] * y + self.z2;
        self.z2 = c.b[2] * x - c.a[1] * y;
        y
    }
}
#[derive(Clone, Copy, Debug)]
struct Coefficients {
    pink: [Biquad; SECTIONS],
    red: Biquad,
    gray: [Biquad; SECTIONS],
    gain: [f64; 6],
}
impl Coefficients {
    fn power(&self, color: usize, omega: f64) -> f64 {
        let difference = 4.0 * (omega / 2.0).sin().powi(2);
        match color {
            0 | 6 => 1.0,
            1 => self.pink.iter().map(|c| c.power(omega)).product(),
            2 => self.red.power(omega),
            3 => difference,
            4 => self.power(1, omega) * difference,
            5 => self.gray.iter().map(|c| c.power(omega)).product(),
            _ => unreachable!(),
        }
    }
    fn design(rate: u32) -> Self {
        let fs = f64::from(rate);
        let ratio = (4.0 * fs).powf(1.0 / SECTIONS as f64);
        let pink = std::array::from_fn(|i| {
            let pole = (-TAU * ratio.powf(i as f64) / fs).exp();
            let zero = (-TAU * ratio.powf(i as f64 + 0.5) / fs).exp();
            Biquad {
                b: [1.0, -zero, 0.0],
                a: [-pole, 0.0],
            }
        });
        let mut result = Self {
            pink,
            red: Biquad {
                b: [1.0, 0.0, 0.0],
                a: [-(-TAU * 5.0 / fs).exp(), 0.0],
            },
            gray: gray::design(rate),
            gain: [1.0; 6],
        };
        // Log-frequency quadrature resolves sub-Hz poles and Nyquist together.
        // This is compile-time bounded work, not a runtime gain estimator.
        const STEPS: usize = 8192;
        let start = 1e-9;
        let ratio = (PI / start).powf(1.0 / STEPS as f64);
        for color in 0..6 {
            let mut omega = start;
            let mut previous = result.power(color, omega);
            let mut integral = previous * start;
            for _ in 0..STEPS {
                let next = omega * ratio;
                let power = result.power(color, next);
                integral += (previous + power) * 0.5 * (next - omega);
                omega = next;
                previous = power;
            }
            result.gain[color] = NOMINAL_RMS / (integral / PI).sqrt();
        }
        result
    }
}
fn coefficients(rate: u32) -> Coefficients {
    static CACHE: OnceLock<Mutex<Vec<(u32, Coefficients)>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(Vec::with_capacity(16)))
        .lock()
        .unwrap();
    if let Some((_, coefficients)) = cache.iter().find(|(fs, _)| *fs == rate) {
        return *coefficients;
    }
    let result = Coefficients::design(rate);
    if cache.len() == 16 {
        cache.remove(0);
    }
    cache.push((rate, result));
    result
}

/// Actual configured noise filter response in dB relative to 1 kHz. Intended
/// for control-thread displays and analysis: this may build/cache coefficients.
/// White/black are flat. Invalid ports, rates or frequencies return `None`.
pub fn colored_noise_response_db(port: &str, frequency_hz: f64, sample_rate: u32) -> Option<f64> {
    let color = OUTPUTS.iter().position(|name| *name == port)?;
    if !(8000..=192000).contains(&sample_rate)
        || !frequency_hz.is_finite()
        || frequency_hz <= 0.0
        || frequency_hz >= f64::from(sample_rate) / 2.0
    {
        return None;
    }
    let c = coefficients(sample_rate);
    let omega = TAU * frequency_hz / f64::from(sample_rate);
    let reference = TAU * 1000.0 / f64::from(sample_rate);
    Some(10.0 * (c.power(color, omega) / c.power(color, reference)).log10())
}

#[derive(Clone, Copy, Debug)]
struct Random {
    state: u64,
    spare: Option<f64>,
}
impl Random {
    fn new(seed: u32, stream: u64) -> Self {
        // SplitMix finalization diffuses neighboring user seeds and stream IDs.
        let mut state = u64::from(seed)
            .wrapping_add(stream)
            .wrapping_add(0x9e37_79b9_7f4a_7c15);
        state = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        state = (state ^ (state >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        state ^= state >> 31;
        Self {
            state: state.max(1),
            spare: None,
        }
    }
    fn uniform(&mut self) -> f64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let bits = self.state.wrapping_mul(2_685_821_657_736_338_717);
        ((bits >> 12) as f64 + 0.5) / 4_503_599_627_370_496.0
    }
    fn gaussian(&mut self) -> f64 {
        if let Some(value) = self.spare.take() {
            return value;
        }
        let radius = (-2.0 * self.uniform().ln()).sqrt();
        let (sin, cos) = (TAU * self.uniform()).sin_cos();
        self.spare = Some(radius * sin);
        radius * cos
    }
}
#[derive(Clone, Copy, Debug)]
struct State {
    gaussian: Random,
    uniform: Random,
    pink: [History; SECTIONS],
    red: History,
    gray: [History; SECTIONS],
    previous_white: f64,
    previous_pink: f64,
}
impl State {
    fn new(seed: u32) -> Self {
        Self {
            gaussian: Random::new(seed, 0),
            uniform: Random::new(seed, 0xd1b5_4a32_d192_ed03),
            pink: [History::default(); SECTIONS],
            red: History::default(),
            gray: [History::default(); SECTIONS],
            previous_white: 0.0,
            previous_pink: 0.0,
        }
    }
}
#[derive(Debug)]
pub(crate) struct ColoredNoise {
    seed: u32,
    level: f64,
    coefficients: Coefficients,
    state: State,
}
impl ColoredNoise {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if !["seed", "level"].contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let seed = parameter(node, "seed", 1.0, 0.0, 16777215.0)?;
        if seed.fract() != 0.0 {
            return Err(format!(
                "node '{}': parameter 'seed' must be an integer",
                node.id
            ));
        }
        Ok(Self {
            seed: seed as u32,
            level: f64::from(parameter(node, "level", 1.0, 0.0, 2.0)?),
            coefficients: coefficients(rate),
            state: State::new(seed as u32),
        })
    }
    pub(crate) fn reset_state(&mut self) {
        self.state = State::new(self.seed);
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.seed == old.seed {
            self.state = old.state;
        }
    }
    #[inline]
    fn values(&mut self) -> [f32; 7] {
        let c = &self.coefficients;
        let s = &mut self.state;
        let white = s.gaussian.gaussian();
        let mut pink = s.gaussian.gaussian();
        for (history, coefficient) in s.pink.iter_mut().zip(c.pink) {
            pink = history.process(coefficient, pink);
        }
        let red = s.red.process(c.red, white);
        let violet = white - s.previous_white;
        let blue = pink - s.previous_pink;
        s.previous_white = white;
        s.previous_pink = pink;
        let mut gray = white;
        for (history, coefficient) in s.gray.iter_mut().zip(c.gray) {
            gray = history.process(coefficient, gray);
        }
        let raw = [white, pink, red, violet, blue, gray];
        let mut output = [0.0; 7];
        for i in 0..6 {
            output[i] = (raw[i] * c.gain[i] * self.level) as f32;
        }
        output[6] = ((s.uniform.uniform() * 2.0 - 1.0) * 5.0 * self.level) as f32;
        output
    }
    pub(crate) fn process(&mut self, outputs: &mut [PortFrame]) {
        for (output, value) in outputs.iter_mut().zip(self.values()) {
            *output = PortFrame::mono(value);
        }
    }
}

#[cfg(test)]
mod tests;
