//! Numerical probe of the production Rust kernel, using the frozen C++ protocol.
//! Original OpenPatch harness, MIT licensed.
#[allow(dead_code)]
#[path = "../../crates/openpatch-core/src/cross_matter/kernel.rs"]
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
    let blocks = read!(usize);
    let size = read!(usize);
    if blocks == 0 || blocks > 20000 {
        return Err("invalid block count".into());
    }
    if size != 60 {
        return Err("unsupported block size".into());
    }
    let mut modulator = Box::new(kernel::Modulator::new(96000.0));
    let mut frames = vec![[0i16; 2]; size];
    let mut output = vec![[0i16; 2]; size];
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut guarded_blocks = Vec::new();
    for block in 0..blocks {
        let easter_egg = read!(i32);
        let carrier_shape = read!(i32);
        if !(0..=3).contains(&carrier_shape) {
            return Err("invalid carrier shape".into());
        }
        if easter_egg != 0 && easter_egg != 1 {
            return Err("invalid mode".into());
        }
        let p = &mut modulator.parameters;
        p.carrier_shape = carrier_shape;
        p.channel_drive[0] = read!(f32);
        p.channel_drive[1] = read!(f32);
        p.modulation_algorithm = read!(f32);
        p.modulation_parameter = read!(f32);
        p.frequency_shift_pot = read!(f32);
        p.frequency_shift_cv = read!(f32);
        p.phase_shift = read!(f32);
        p.note = read!(f32);
        modulator.easter_egg = easter_egg != 0;
        for frame in frames.iter_mut() {
            frame[0] = read!(i16);
            frame[1] = read!(i16);
        }
        let guarded_before = modulator.guarded_reads;
        modulator.process(&frames, &mut output);
        if modulator.guarded_reads != guarded_before {
            guarded_blocks.push(block.to_string());
        }
        let line: Vec<String> = output
            .iter()
            .map(|frame| format!("{} {}", frame[0], frame[1]))
            .collect();
        writeln!(out, "{}", line.join(" "))?;
    }
    out.flush()?;
    // Blocks in which the source would read outside a table (undefined
    // behaviour); compare.py excludes them and everything after them from
    // the exact domain and reports them separately.
    eprintln!("guarded_blocks {}", guarded_blocks.join(" "));
    eprintln!("guarded_reads {}", modulator.guarded_reads);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
