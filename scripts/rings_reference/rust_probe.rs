//! Numerical probe of the production Rust kernel, using the frozen C++ protocol.
//! Original OpenPatch harness, MIT licensed.
#[allow(dead_code)]
#[path = "../../crates/openpatch-core/src/string_field/kernel.rs"]
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
    let seed = read!(u32);
    if blocks == 0 || blocks > 200_000 {
        return Err("invalid block count".into());
    }
    if size != kernel::MAX_BLOCK_SIZE {
        return Err("unsupported block size".into());
    }
    let mut processor = Box::new(kernel::StringFieldKernel::new());
    processor.seed_random(seed);
    let mut frames = [0i16; kernel::MAX_BLOCK_SIZE];
    let mut output = [[0i16; 2]; kernel::MAX_BLOCK_SIZE];
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut guarded_blocks = Vec::new();
    for block in 0..blocks {
        let polyphony = read!(i32);
        let model = read!(i32);
        if !(1..=kernel::MAX_POLYPHONY).contains(&polyphony) || !(0..=5).contains(&model) {
            return Err("invalid polyphony or model".into());
        }
        let strum = read!(i32) != 0;
        let internal_exciter = read!(i32) != 0;
        let internal_strum = read!(i32) != 0;
        let internal_note = read!(i32) != 0;
        let chord = read!(i32);
        if !(0..kernel::NUM_CHORDS).contains(&chord) {
            return Err("invalid chord".into());
        }
        let performance_state = kernel::PerformanceState {
            strum,
            internal_exciter,
            internal_strum,
            internal_note,
            chord,
            tonic: read!(f32),
            note: read!(f32),
            fm: read!(f32),
        };
        let patch = kernel::Patch {
            structure: read!(f32),
            brightness: read!(f32),
            damping: read!(f32),
            position: read!(f32),
        };
        if polyphony != processor.polyphony() {
            processor.set_polyphony(polyphony);
        }
        processor.set_model(kernel::ResonatorModel::from_index(model));
        for frame in frames.iter_mut() {
            *frame = read!(i16);
        }
        let guarded_before = processor.guarded_reads();
        processor.process(performance_state, &patch, &frames, &mut output);
        if processor.guarded_reads() != guarded_before {
            guarded_blocks.push(block.to_string());
        }
        let line: Vec<String> = output
            .iter()
            .map(|frame| format!("{} {}", frame[0], frame[1]))
            .collect();
        writeln!(out, "{} {}", line.join(" "), u8::from(processor.last_strum()))?;
    }
    out.flush()?;
    // Blocks in which the source would read outside an object (undefined
    // behaviour); compare.py excludes them and everything after them from
    // the exact domain and reports them separately.
    eprintln!("guarded_blocks {}", guarded_blocks.join(" "));
    eprintln!("guarded_reads {}", processor.guarded_reads());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
