//! Numerical probe of the production Rust kernel, using the frozen C++ protocol.
//! Original OpenPatch harness, MIT licensed.
#[allow(dead_code)]
#[path = "../../crates/openpatch-core/src/grain_weather/kernel.rs"]
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
    let prepare_calls = read!(usize);
    let seed = read!(u32);
    if blocks == 0 || blocks > 40000 {
        return Err("invalid block count".into());
    }
    if size != kernel::MAX_BLOCK_SIZE {
        return Err("unsupported block size".into());
    }
    if !(1..=64).contains(&prepare_calls) {
        return Err("invalid prepare count".into());
    }
    let mut processor = Box::new(kernel::GranularProcessor::new());
    processor.seed_random(seed);
    let mut frames = [[0i16; 2]; kernel::MAX_BLOCK_SIZE];
    let mut output = [[0i16; 2]; kernel::MAX_BLOCK_SIZE];
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut guarded_blocks = Vec::new();
    for block in 0..blocks {
        let mode = read!(i32);
        let quality = read!(i32);
        if !(0..=3).contains(&mode) || !(0..=3).contains(&quality) {
            return Err("invalid mode".into());
        }
        let freeze = read!(i32) != 0;
        let trigger = read!(i32) != 0;
        let gate = read!(i32) != 0;
        let p = &mut processor.parameters;
        p.freeze = freeze;
        p.trigger = trigger;
        p.gate = gate;
        p.position = read!(f32);
        p.size = read!(f32);
        p.pitch = read!(f32);
        p.density = read!(f32);
        p.texture = read!(f32);
        p.dry_wet = read!(f32);
        p.stereo_spread = read!(f32);
        p.feedback = read!(f32);
        p.reverb = read!(f32);
        processor.set_playback_mode(kernel::PlaybackMode::from_index(mode));
        processor.set_quality(quality);
        for frame in frames.iter_mut() {
            frame[0] = read!(i16);
            frame[1] = read!(i16);
        }
        let guarded_before = processor.guarded_reads();
        processor.process(&frames, &mut output);
        for _ in 0..prepare_calls {
            processor.prepare();
        }
        if processor.guarded_reads() != guarded_before {
            guarded_blocks.push(block.to_string());
        }
        let line: Vec<String> = output
            .iter()
            .map(|frame| format!("{} {}", frame[0], frame[1]))
            .collect();
        writeln!(out, "{}", line.join(" "))?;
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
