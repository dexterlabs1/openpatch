//! Standalone adapter for the exact production kernel; no Cargo dependency.
#[path = "../../crates/openpatch-core/src/event_fork/kernel.rs"]
#[allow(dead_code)]
mod kernel;
use std::io::{self, BufRead, BufWriter, Write};

fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let seed: u32 = lines.next().unwrap().unwrap().parse().unwrap();
    let mut kernel = kernel::Kernel::new(seed);
    let mut out = BufWriter::new(io::stdout().lock());
    for i in 0..256 {
        writeln!(out, "[\"adc\",{},{}]", i, kernel::adc_threshold(i as u8)).unwrap();
    }
    let mut frame = 0_u64;
    for line in lines {
        let values: Vec<u32> = line
            .unwrap()
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        assert_eq!(values.len(), 9);
        assert!(values[0] != 0);
        assert!(values[3] <= 65535 && values[4] <= 65535);
        assert!([1, 2, 5, 6, 7, 8].iter().all(|&i| values[i] <= 1));
        for _ in 0..values[0] {
            let s = kernel.process(
                [values[1] != 0, values[2] != 0],
                [values[3] as u16, values[4] as u16],
                [values[5] != 0, values[6] != 0],
                [values[7] != 0, values[8] != 0],
            );
            writeln!(
                out,
                "[{},{},{},{},{},{},{},{},{},{},{},{},{},{}]",
                frame,
                u8::from(s.outputs[0][0]),
                u8::from(s.outputs[0][1]),
                u8::from(s.outputs[1][0]),
                u8::from(s.outputs[1][1]),
                u8::from(s.previous_state[0]),
                u8::from(s.previous_state[1]),
                u8::from(s.input_state[0]),
                u8::from(s.input_state[1]),
                s.led_state[0],
                s.led_state[1],
                s.led_gate_delay[0],
                s.led_gate_delay[1],
                s.rng_state
            )
            .unwrap();
            frame += 1;
        }
    }
}
