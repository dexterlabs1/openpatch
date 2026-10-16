//! Original Rust implementation of Fundamental Noise's seven output DSP roles.
//! The reference is the pinned Noise.cpp linked below. Random stream seeding is
//! an OpenPatch policy; controlled draws are injectable for conditional parity.
//! FFT plans and work buffers are prepared at graph compilation, not on audio.
use crate::host_ports::HostPortSnapshot;
use crate::native_float::{rack_add, rack_div, rack_mul, rack_sub};
use crate::Node;
use rustfft::{num_complex::Complex32, Fft, FftPlanner};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;

pub(crate) const OUTPUTS: [&str; 7] = ["white", "pink", "red", "violet", "blue", "gray", "black"];
const N: usize = 1024;

pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_noise","title":"STOCK NOISE","model":"functional",
        "description":"Seven independent stock noise output ports with connection-dependent processing. No parameters or inputs.",
        "inputs":[],"params":{}})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Noise.cpp");
    module["compatibility"] = json!("independent Rust source-behavior implementation with controlled normal/uniform draw parity; seeded default stream is OpenPatch policy, not Rack process-global PRNG identity");
    module["polyphony"]["version_2"] = json!("seven independent mono outputs; no inputs; each output's actual cable, selected-output or tap consumer controls source processing");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["auxiliary_inputs"] = json!("no inputs");
    module["polyphony"]["retired_voices"] =
        json!("mono state freezes independently by output consumer; no voice retirement");
    module["light_names"] = json!([]);
    module["consumer_policy"] = json!("white-family normal draw when any of white/red/violet/gray is connected; pink Voss draw when pink/blue connected; black uniform draw last when connected; red/violet/blue/gray histories advance only on their own connections");
    module["gray_policy"] = json!("1024 connected gray samples per block; source packed-bin weighting uses sample_rate*k/(2*1024), DC and Nyquist zero; inverse FFT uses preweighted 1/1024 normalization; first block appears on connected gray sample 1024; unused source scratch upper half is initialized by the reference oracle erratum and has no inverse output effect");
    module["initialization_policy"] = json!("no source onReset callback or controls; Initialize preserves PRNG, pink/red/difference and gray block histories and common raw ports");
    module["state_adoption"] = json!("compatible receiving incarnation, sample rate and draw-provider configuration retain PRNG and every DSP history; changed controlled tables start fresh; new incarnation starts fresh; common host port lifecycle retains raw storage separately");
    module["arithmetic_policy"] = json!("source-ordered f32 red and difference operations with Rack-style signed FTZ/DAZ; rustfft 6.4.1 complex FFT computes real packed-bin equivalent with separate declared FFT tolerance");
    for (index, output) in module["output_metadata"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        output["maximum_channels"] = json!(1);
        output["description"] = json!(match index {
            0 => "0 dB/octave power density",
            1 => "-3 dB/octave power density",
            2 => "-6 dB/octave power density",
            3 => "+6 dB/octave power density",
            4 => "+3 dB/octave power density",
            5 => "Psychoacoustic equal loudness",
            _ => "Uniform random numbers",
        });
    }
}

/// Two source-level calls. A single uniform cursor covers Voss updates then
/// black; normal is an independent controlled stream in the oracle.
trait StockNoiseDraws {
    fn normal(&mut self) -> f32;
    fn uniform(&mut self) -> f32;
    fn uniform_pink(&mut self) -> f32 {
        self.uniform()
    }
    fn uniform_black(&mut self) -> f32 {
        self.uniform()
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct StockNoiseDrawEvent {
    pub kind: &'static str,
    pub bits: u32,
}

/// Control-thread view of actual source histories. Bit arrays preserve signed
/// zero and let the independent oracle compare block startup without warmup.
#[doc(hidden)]
#[derive(Clone, Debug, serde::Serialize)]
pub struct StockNoiseSnapshot {
    pub red_x_bits: u32,
    pub red_y_bits: u32,
    pub last_white_bits: u32,
    pub last_pink_bits: u32,
    pub pink_frame: i32,
    pub pink_values_bits: [u32; 8],
    pub gray_frame: usize,
    pub gray_input_bits: Vec<u32>,
    pub gray_output_bits: Vec<u32>,
    pub black_divider_clock: u32,
    pub black_divider_division: u32,
    pub light_bits: Vec<u32>,
    pub normal_draws: u64,
    pub uniform_draws: u64,
    pub last_normal_bits: u32,
    pub last_uniform_bits: u32,
    pub draw_events: Vec<StockNoiseDrawEvent>,
}

#[derive(Debug)]
struct ControlledDraws {
    normal_bits: Arc<[u32]>,
    uniform_bits: Arc<[u32]>,
    normal_digest: [u8; 32],
    uniform_digest: [u8; 32],
    normal_cursor: u64,
    uniform_cursor: u64,
}
impl ControlledDraws {
    fn normal(&mut self) -> f32 {
        let bits = self.normal_bits[self.normal_cursor as usize % self.normal_bits.len()];
        self.normal_cursor += 1;
        f32::from_bits(bits)
    }
    fn uniform(&mut self) -> f32 {
        let bits = self.uniform_bits[self.uniform_cursor as usize % self.uniform_bits.len()];
        self.uniform_cursor += 1;
        f32::from_bits(bits)
    }
}

#[derive(Debug)]
enum Stream {
    Seeded(SeededDraws),
    Controlled(ControlledDraws),
}

#[derive(Debug)]
struct DrawProvider {
    stream: Stream,
    normal_draws: u64,
    uniform_draws: u64,
    last_normal_bits: u32,
    last_uniform_bits: u32,
    events: [StockNoiseDrawEvent; 10],
    event_count: u8,
}
impl DrawProvider {
    fn digest(bits: &[u32]) -> [u8; 32] {
        let mut hash = Sha256::new();
        for word in bits {
            hash.update(word.to_le_bytes());
        }
        hash.finalize().into()
    }
    fn new(id: &str) -> Self {
        Self {
            stream: Stream::Seeded(SeededDraws::new(id)),
            normal_draws: 0,
            uniform_draws: 0,
            last_normal_bits: 0,
            last_uniform_bits: 0,
            events: [StockNoiseDrawEvent::default(); 10],
            event_count: 0,
        }
    }
    fn begin_frame(&mut self) {
        self.event_count = 0;
    }
    fn controlled(
        &mut self,
        normal_bits: &[u32],
        uniform_bits: &[u32],
        normal_start: u64,
        uniform_start: u64,
    ) -> bool {
        if self.normal_draws != 0
            || self.uniform_draws != 0
            || normal_bits.is_empty()
            || uniform_bits.is_empty()
        {
            return false;
        }
        self.stream = Stream::Controlled(ControlledDraws {
            normal_bits: Arc::from(normal_bits),
            uniform_bits: Arc::from(uniform_bits),
            normal_digest: Self::digest(normal_bits),
            uniform_digest: Self::digest(uniform_bits),
            normal_cursor: normal_start,
            uniform_cursor: uniform_start,
        });
        self.normal_draws = normal_start;
        self.uniform_draws = uniform_start;
        self.last_normal_bits = if normal_start == 0 {
            0
        } else {
            normal_bits[(normal_start - 1) as usize % normal_bits.len()]
        };
        self.last_uniform_bits = if uniform_start == 0 {
            0
        } else {
            uniform_bits[(uniform_start - 1) as usize % uniform_bits.len()]
        };
        true
    }
    fn same_configuration(&self, old: &Self) -> bool {
        match (&self.stream, &old.stream) {
            (Stream::Seeded(_), Stream::Seeded(_)) => true,
            (Stream::Controlled(a), Stream::Controlled(b)) => {
                a.normal_bits.len() == b.normal_bits.len()
                    && a.uniform_bits.len() == b.uniform_bits.len()
                    && a.normal_digest == b.normal_digest
                    && a.uniform_digest == b.uniform_digest
            }
            _ => false,
        }
    }
    fn adopt_runtime(&mut self, old: &Self) {
        match (&mut self.stream, &old.stream) {
            (Stream::Seeded(receiving), Stream::Seeded(previous)) => *receiving = *previous,
            (Stream::Controlled(receiving), Stream::Controlled(previous)) => {
                receiving.normal_cursor = previous.normal_cursor;
                receiving.uniform_cursor = previous.uniform_cursor;
            }
            _ => unreachable!("draw-provider configuration checked before adoption"),
        }
        self.normal_draws = old.normal_draws;
        self.uniform_draws = old.uniform_draws;
        self.last_normal_bits = old.last_normal_bits;
        self.last_uniform_bits = old.last_uniform_bits;
        self.event_count = 0;
    }
    fn record(&mut self, kind: &'static str, value: f32) -> f32 {
        let index = usize::from(self.event_count);
        debug_assert!(index < self.events.len());
        self.events[index] = StockNoiseDrawEvent {
            kind,
            bits: value.to_bits(),
        };
        self.event_count += 1;
        if kind == "normal" {
            self.normal_draws += 1;
            self.last_normal_bits = value.to_bits();
        } else {
            self.uniform_draws += 1;
            self.last_uniform_bits = value.to_bits();
        }
        value
    }
    fn draw_uniform(&mut self, kind: &'static str) -> f32 {
        let value = match &mut self.stream {
            Stream::Seeded(s) => s.uniform(),
            Stream::Controlled(s) => s.uniform(),
        };
        self.record(kind, value)
    }
}
impl StockNoiseDraws for DrawProvider {
    fn normal(&mut self) -> f32 {
        let value = match &mut self.stream {
            Stream::Seeded(s) => s.normal(),
            Stream::Controlled(s) => s.normal(),
        };
        self.record("normal", value)
    }
    fn uniform(&mut self) -> f32 {
        self.draw_uniform("uniform")
    }
    fn uniform_pink(&mut self) -> f32 {
        self.draw_uniform("pink")
    }
    fn uniform_black(&mut self) -> f32 {
        self.draw_uniform("black")
    }
}

#[derive(Debug, Clone, Copy)]
struct SeededDraws {
    state: u64,
    spare: Option<f32>,
}
impl SeededDraws {
    fn new(id: &str) -> Self {
        // Independent per-module policy for ordinary OpenPatch operation.
        let mut state = 0xcbf2_9ce4_8422_2325_u64;
        for byte in id.bytes() {
            state = (state ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
        Self {
            state: state.max(1),
            spare: None,
        }
    }
    fn unit(&mut self) -> f32 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let bits = self.state.wrapping_mul(2_685_821_657_736_338_717);
        // Open interval avoids ln(0) and a rounded 1.0 in Box-Muller.
        ((bits >> 40) as u32).clamp(1, 16_777_215) as f32 / 16_777_216.0
    }
}
impl StockNoiseDraws for SeededDraws {
    fn normal(&mut self) -> f32 {
        if let Some(spare) = self.spare.take() {
            return spare;
        }
        let radius = (-2.0 * self.unit().ln()).sqrt();
        let angle = std::f32::consts::TAU * self.unit();
        let (sin, cos) = angle.sin_cos();
        self.spare = Some(radius * sin);
        radius * cos
    }
    fn uniform(&mut self) -> f32 {
        self.unit()
    }
}

struct Gray {
    input: [f32; N],
    output: [f32; N],
    frame: usize,
    spectrum: Vec<Complex32>,
    forward_scratch: Vec<Complex32>,
    inverse_scratch: Vec<Complex32>,
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    sample_time: f32,
}
impl fmt::Debug for Gray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gray")
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}
impl Gray {
    fn new(rate: u32) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(N);
        let inverse = planner.plan_fft_inverse(N);
        Self {
            input: [0.0; N],
            output: [0.0; N],
            frame: 0,
            spectrum: vec![Complex32::new(0.0, 0.0); N],
            forward_scratch: vec![Complex32::new(0.0, 0.0); forward.get_inplace_scratch_len()],
            inverse_scratch: vec![Complex32::new(0.0, 0.0); inverse.get_inplace_scratch_len()],
            forward,
            inverse,
            sample_time: 1.0 / rate as f32,
        }
    }
    fn adopt(&mut self, old: &Self) {
        self.input = old.input;
        self.output = old.output;
        self.frame = old.frame;
    }
    fn process(&mut self, white: f32) -> f32 {
        self.input[self.frame] = white;
        self.frame += 1;
        if self.frame == N {
            self.frame = 0;
            for (slot, sample) in self.spectrum.iter_mut().zip(&self.input) {
                *slot = Complex32::new(*sample, 0.0);
            }
            self.forward
                .process_with_scratch(&mut self.spectrum, &mut self.forward_scratch);
            // Rack RealFFT packs DC at 0, Nyquist at 1, then (real,imag)
            // pairs. Source i=0 zeroes both DC and Nyquist. Only bins 1..511
            // influence inverse output; source i>=512 weights unused scratch.
            self.spectrum[0] = Complex32::new(0.0, 0.0);
            self.spectrum[N / 2] = Complex32::new(0.0, 0.0);
            for k in 1..N / 2 {
                let frequency = rack_mul(
                    rack_div(rack_div(rack_div(1.0, self.sample_time), 2.0), N as f32),
                    k as f32,
                );
                let amp = if (80.0..=20_000.0).contains(&frequency) {
                    let f2 = rack_mul(frequency, frequency);
                    let left = rack_add(424.36, f2);
                    let middle = rack_mul(rack_add(11_599.3, f2), rack_add(544_496.0, f2)).sqrt();
                    let numerator = rack_mul(rack_mul(left, middle), rack_add(148_693_636.0, f2));
                    let denominator = rack_mul(rack_mul(148_693_636.0, f2), f2);
                    rack_div(numerator, denominator)
                } else {
                    0.0
                };
                let scale = rack_div(amp, N as f32);
                self.spectrum[k].re = rack_mul(self.spectrum[k].re, scale);
                self.spectrum[k].im = rack_mul(self.spectrum[k].im, scale);
                self.spectrum[N - k] = self.spectrum[k].conj();
            }
            self.inverse
                .process_with_scratch(&mut self.spectrum, &mut self.inverse_scratch);
            for (out, value) in self.output.iter_mut().zip(&self.spectrum) {
                *out = value.re;
            }
        }
        self.output[self.frame]
    }
}

#[derive(Debug)]
pub(crate) struct StockNoise {
    rate: u32,
    draws: DrawProvider,
    pink_frame: i32,
    pink_values: [f32; 8],
    red_last_input: f32,
    red_last_output: f32,
    last_white: f32,
    last_pink: f32,
    gray: Box<Gray>,
}
impl StockNoise {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if node.kind != "stock_noise" {
            return Err(format!("node '{}': wrong stock noise kind", node.id));
        }
        if let Some(key) = node.params.keys().next() {
            return Err(format!("node '{}': unknown parameter '{key}'", node.id));
        }
        if !(8000..=192000).contains(&rate) {
            return Err("stock_noise sample rate must be in 8000..=192000 Hz".into());
        }
        Ok(Self {
            rate,
            draws: DrawProvider::new(&node.id),
            pink_frame: -1,
            pink_values: [0.0; 8],
            red_last_input: 0.0,
            red_last_output: 0.0,
            last_white: 0.0,
            last_pink: 0.0,
            gray: Box::new(Gray::new(rate)),
        })
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.rate != old.rate || !self.draws.same_configuration(&old.draws) {
            return;
        }
        self.draws.adopt_runtime(&old.draws);
        self.pink_frame = old.pink_frame;
        self.pink_values = old.pink_values;
        self.red_last_input = old.red_last_input;
        self.red_last_output = old.red_last_output;
        self.last_white = old.last_white;
        self.last_pink = old.last_pink;
        self.gray.adopt(&old.gray);
    }
    pub(crate) fn initialize_state(&mut self) {
        // Fundamental Noise has no onReset. The diagnostic event log describes
        // the current callback, so a no-sample Initialize has no draw events.
        self.draws.begin_frame();
    }
    pub(crate) fn begin_frame(&mut self) {
        self.draws.begin_frame();
    }
    pub(crate) fn snapshot(&self) -> StockNoiseSnapshot {
        StockNoiseSnapshot {
            red_x_bits: self.red_last_input.to_bits(),
            red_y_bits: self.red_last_output.to_bits(),
            last_white_bits: self.last_white.to_bits(),
            last_pink_bits: self.last_pink.to_bits(),
            pink_frame: self.pink_frame,
            pink_values_bits: self.pink_values.map(f32::to_bits),
            gray_frame: self.gray.frame,
            gray_input_bits: self.gray.input.iter().map(|x| x.to_bits()).collect(),
            gray_output_bits: self.gray.output.iter().map(|x| x.to_bits()).collect(),
            // Source declares an unused ClockDivider with these SDK defaults.
            black_divider_clock: 0,
            black_divider_division: 1,
            light_bits: Vec::new(),
            normal_draws: self.draws.normal_draws,
            uniform_draws: self.draws.uniform_draws,
            last_normal_bits: self.draws.last_normal_bits,
            last_uniform_bits: self.draws.last_uniform_bits,
            draw_events: self.draws.events[..usize::from(self.draws.event_count)].to_vec(),
        }
    }
    pub(crate) fn set_controlled_draws(
        &mut self,
        normal_bits: &[u32],
        uniform_bits: &[u32],
        normal_start: u64,
        uniform_start: u64,
    ) -> bool {
        self.draws
            .controlled(normal_bits, uniform_bits, normal_start, uniform_start)
    }
    pub(crate) fn process(&mut self, outputs: &mut [HostPortSnapshot]) {
        // The common host has already reconciled actual consumers and bypass.
        let mask = outputs.iter().enumerate().fold(0u8, |mask, (i, output)| {
            mask | ((output.channels != 0) as u8) << i
        });
        let Self {
            draws,
            pink_frame,
            pink_values,
            red_last_input,
            red_last_output,
            last_white,
            last_pink,
            gray,
            ..
        } = self;
        Self::process_state(
            mask,
            outputs,
            draws,
            pink_frame,
            pink_values,
            red_last_input,
            red_last_output,
            last_white,
            last_pink,
            gray,
        );
    }
    // Keep the pinned source's decimal coefficient spelling and f32 rounding.
    #[allow(clippy::too_many_arguments, clippy::excessive_precision)]
    fn process_state<D: StockNoiseDraws>(
        mask: u8,
        outputs: &mut [HostPortSnapshot],
        draws: &mut D,
        pink_frame: &mut i32,
        pink_values: &mut [f32; 8],
        red_last_input: &mut f32,
        red_last_output: &mut f32,
        last_white: &mut f32,
        last_pink: &mut f32,
        gray: &mut Gray,
    ) {
        let gain = rack_div(5.0, 2.0_f32.sqrt());
        if mask & 0b0010_1101 != 0 {
            let white = draws.normal();
            outputs[0].values[0] = rack_mul(white, gain);
            if mask & (1 << 2) != 0 {
                let red = rack_sub(
                    rack_add(
                        rack_mul(0.00425611, white),
                        rack_mul(0.00425611, *red_last_input),
                    ),
                    rack_mul(-0.99148778, *red_last_output),
                );
                *red_last_input = white;
                *red_last_output = red;
                outputs[2].values[0] = rack_mul(rack_div(red, 0.0645), gain);
            }
            if mask & (1 << 3) != 0 {
                let violet = rack_div(rack_sub(white, *last_white), 1.41);
                *last_white = white;
                outputs[3].values[0] = rack_mul(violet, gain);
            }
            if mask & (1 << 5) != 0 {
                outputs[5].values[0] = rack_mul(rack_div(gray.process(white), 1.67), gain);
            }
        }
        if mask & 0b0001_0010 != 0 {
            let old_frame = *pink_frame;
            *pink_frame += 1;
            if *pink_frame >= 256 {
                *pink_frame = 0;
            }
            let difference = old_frame ^ *pink_frame;
            let mut sum = 0.0;
            for (i, value) in pink_values.iter_mut().enumerate() {
                if difference & (1 << i) != 0 {
                    *value = rack_sub(draws.uniform_pink(), 0.5);
                }
                sum = rack_add(sum, *value);
            }
            let pink = rack_div(sum, 0.816);
            outputs[1].values[0] = rack_mul(pink, gain);
            if mask & (1 << 4) != 0 {
                let blue = rack_div(rack_sub(pink, *last_pink), 0.705);
                *last_pink = pink;
                outputs[4].values[0] = rack_mul(blue, gain);
            }
        }
        if mask & (1 << 6) != 0 {
            outputs[6].values[0] = rack_sub(rack_mul(draws.uniform_black(), 10.0), 5.0);
        }
    }
}
