//! Numerical probe of the production Rust files, using the frozen C++ protocol.
#[path = "../../crates/openpatch-core/src/tidal_functions/extractor.rs"]
mod extractor;
#[path = "../../crates/openpatch-core/src/tidal_functions/kernel.rs"]
mod kernel;

use std::io::{self, Read, Write};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let mut words = input.split_whitespace();
    macro_rules! read {
        ($ty:ty) => {
            words.next().ok_or("missing input")?.parse::<$ty>()?
        };
    }
    let rate = read!(f32);
    let size = read!(usize);
    let blocks = read!(usize);
    let mode = read!(u8);
    let output_mode = read!(u8);
    let range = read!(u8);
    let trigger_patched = read!(u8) != 0;
    let clock_patched = read!(u8) != 0;
    let gate_period = read!(usize);
    let gate_high = read!(usize);
    let clock_period = read!(usize);
    let clock_high = read!(usize);
    let ratio = read!(f32);
    let denominator = read!(i32);
    let clock_start = read!(usize);
    let mut previous_clock = read!(u8) != 0;
    if !(rate > 0.0
        && size > 0
        && size <= 64
        && blocks <= 20000
        && mode < 3
        && output_mode < 4
        && range < 2)
    {
        return Err("invalid dimensions or enum".into());
    }
    let mut generator = kernel::TidalKernel::new();
    let mut extractor = extractor::RampExtractor::new(rate, 40.0 / rate);
    let mut previous_gate = false;
    let mut gates = vec![0; size];
    let mut clocks = vec![0; size];
    let mut ramp = vec![0.0; size];
    let mut output = vec![[0.0; 4]; size];
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for block in 0..blocks {
        let mut frequency = read!(f32);
        let slope = read!(f32);
        let shape = read!(f32);
        let smoothness = read!(f32);
        let shift = read!(f32);
        if read!(u8) != 0 {
            generator.reset_filter();
        }
        if read!(u8) != 0 {
            extractor.reset();
        }
        for i in 0..size {
            let frame = block * size + i;
            let gate = trigger_patched && gate_period != 0 && frame % gate_period < gate_high;
            let clock = clock_patched
                && clock_period != 0
                && frame >= clock_start
                && (frame - clock_start) % clock_period < clock_high;
            gates[i] = flags(previous_gate, gate);
            clocks[i] = flags(previous_clock, clock);
            previous_gate = gate;
            previous_clock = clock;
        }
        if clock_patched {
            frequency = extractor.process(
                range == 1,
                range == 1 && mode == 2,
                ratio,
                denominator,
                &clocks,
                &mut ramp,
            );
        }
        if !frequency.is_finite() {
            return Err("nonfinite frequency".into());
        }
        generator.render(
            mode,
            output_mode,
            range,
            frequency,
            slope,
            shape,
            smoothness,
            shift,
            &gates,
            (clock_patched && !trigger_patched).then_some(ramp.as_slice()),
            &mut output,
        );
        for i in 0..size {
            if !output[i].iter().all(|v| v.is_finite()) {
                return Err("nonfinite output".into());
            }
            let ramp_bits = if clock_patched {
                if !ramp[i].is_finite() {
                    return Err("nonfinite extracted ramp".into());
                }
                ramp[i].to_bits().to_string()
            } else {
                "null".to_string()
            };
            writeln!(stdout, "{{\"frame\":{},\"block\":{},\"frequency_bits\":{},\"gate_flags\":{},\"clock_flags\":{},\"extracted_ramp_bits\":{},\"output_bits\":[{},{},{},{}]}}",
                block * size + i, block, frequency.to_bits(), gates[i], clocks[i], ramp_bits,
                output[i][0].to_bits(), output[i][1].to_bits(), output[i][2].to_bits(), output[i][3].to_bits())?;
        }
    }
    if words.next().is_some() {
        return Err("unexpected trailing input".into());
    }
    stdout.flush()?;
    Ok(())
}

fn flags(previous: bool, current: bool) -> u8 {
    u8::from(current)
        | if current && !previous {
            2
        } else if previous && !current {
            4
        } else {
            0
        }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
