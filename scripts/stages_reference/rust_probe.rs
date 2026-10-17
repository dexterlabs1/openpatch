//! Independent stdin adapter for the actual production Rust kernel.
#[path = "../../crates/openpatch-core/src/tidal_functions/extractor.rs"]
pub mod standalone_extractor;
mod tidal_functions {
    pub use crate::standalone_extractor as extractor;
}
#[path = "../../crates/openpatch-core/src/segment_garden/kernel.rs"]
#[allow(dead_code)]
mod kernel;
use kernel::{Configuration, Output, SegmentGenerator, SegmentType};
use std::io::{self, BufRead, BufWriter, Write};

fn next<T: std::str::FromStr>(input: &mut std::str::SplitWhitespace<'_>) -> T {
    input
        .next()
        .expect("missing token")
        .parse()
        .ok()
        .expect("invalid token")
}
fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let header = lines.next().unwrap().unwrap();
    let mut h = header.split_whitespace();
    let quantized: bool = next::<u32>(&mut h) != 0;
    let mut rng: u32 = next(&mut h);
    let mut notes = [0; 42];
    let mut generators: [SegmentGenerator; 6] = std::array::from_fn(|_| {
        if quantized {
            SegmentGenerator::new_quantized()
        } else {
            SegmentGenerator::new()
        }
    });
    let mut previous = [0_u8; 6];
    let mut frame = 0_usize;
    let mut out = BufWriter::new(io::stdout().lock());
    for line in lines {
        let line = line.unwrap();
        let mut tokens = line.split_whitespace();
        let op = tokens.next().unwrap();
        let slot: usize = next(&mut tokens);
        match op {
            "C" | "I" => {
                let triggered = next::<u32>(&mut tokens) != 0;
                let count: usize = next(&mut tokens);
                let configs: Vec<_> = (0..count)
                    .map(|_| Configuration {
                        kind: match next::<u32>(&mut tokens) {
                            0 => SegmentType::Ramp,
                            1 => SegmentType::Step,
                            2 => SegmentType::Hold,
                            _ => panic!("unsupported kind"),
                        },
                        looped: next::<u32>(&mut tokens) != 0,
                    })
                    .collect();
                if op == "I" {
                    generators[slot]
                        .configure_single(triggered, configs[0])
                        .unwrap();
                } else {
                    generators[slot].configure(triggered, &configs).unwrap();
                }
            }
            "P" => {
                let index = next(&mut tokens);
                let primary = next(&mut tokens);
                let secondary = next(&mut tokens);
                generators[slot]
                    .set_parameters(index, primary, secondary)
                    .unwrap();
            }
            "S" => generators[slot].configure_slave(next(&mut tokens)).unwrap(),
            "B" => {
                let size: usize = next(&mut tokens);
                let blocks: usize = next(&mut tokens);
                let period: usize = next(&mut tokens);
                let high: usize = next(&mut tokens);
                let onset: usize = next(&mut tokens);
                let count: usize = next(&mut tokens);
                let slaves: Vec<usize> = (0..count).map(|_| next(&mut tokens)).collect();
                let mut gates = vec![0; size];
                let mut output = vec![Output::default(); size];
                for _ in 0..blocks {
                    for (i, flags) in gates.iter_mut().enumerate() {
                        let t = frame + i;
                        let level = period != 0 && t >= onset && (t - onset) % period < high;
                        *flags = if level {
                            if previous[slot] & 1 != 0 {
                                1
                            } else {
                                3
                            }
                        } else if previous[slot] & 1 != 0 {
                            4
                        } else {
                            0
                        };
                        previous[slot] = *flags;
                    }
                    for i in 0..36 {
                        generators[slot]
                            .set_note_quantizer_state(i, notes[slot + i])
                            .unwrap();
                    }
                    generators[slot].seed_rng(rng);
                    generators[slot].process(&gates, &mut output);
                    rng = generators[slot].rng_state();
                    for i in 0..36 {
                        notes[slot + i] = generators[slot].note_quantizer_state(i).unwrap();
                    }
                    let mut monitored = Vec::new();
                    for &slave in &slaves {
                        let mut copy = output.clone();
                        generators[slave].seed_rng(rng);
                        generators[slave].process(&gates, &mut copy);
                        rng = generators[slave].rng_state();
                        monitored.push(copy);
                    }
                    for i in 0..size {
                        let mut emit = |slot: usize, v: Output| {
                            assert!(
                                v.value.is_finite() && v.phase.is_finite(),
                                "nonfinite native output"
                            );
                            writeln!(
                                out,
                                "[{},{},{},{},{},{},{}]",
                                frame + i,
                                slot,
                                gates[i],
                                v.value.to_bits(),
                                v.phase.to_bits(),
                                v.segment,
                                rng
                            )
                            .unwrap();
                        };
                        emit(slot, output[i]);
                        for (j, &slave) in slaves.iter().enumerate() {
                            emit(slave, monitored[j][i]);
                        }
                    }
                    frame += size;
                }
            }
            _ => panic!("unknown operation"),
        }
        assert!(tokens.next().is_none());
    }
    out.flush().unwrap();
}
