// SPDX-License-Identifier: MIT
// Original observation adapter for the compiled production openpatch_core crate.
// No copied DSP source, replacement host types, or second filter model.
use openpatch_core::{Node, PortFrame, Rack264VcfProbe};
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufWriter, Write};

fn integer(value: f32, low: u16, high: u16) -> u16 {
    assert!(value.is_finite() && value.fract() == 0.0);
    assert!(value >= f32::from(low) && value <= f32::from(high));
    value as u16
}

fn seed_word(value: &str) -> u64 {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).unwrap()
    } else {
        value.parse().unwrap()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 5);
    let rate: u32 = args[1].parse().expect("sample_rate must be an integer");
    let seed = [seed_word(&args[2]), seed_word(&args[3])];
    let history: f32 = args[4].parse().unwrap();
    let mut node: Node = serde_json::from_value(serde_json::json!({
        "id": "reference", "kind": "rack264_vcf", "params": {}
    }))
    .unwrap();
    let mut module = Rack264VcfProbe::new(&node, rate).unwrap();
    // Explicit recorded assignment; upstream constructor history is undefined.
    module.initialize_reference([history; 16], seed).unwrap();
    let initial = module.snapshot();
    eprintln!("INITIAL_HISTORY_ASSIGNED=16 INITIAL_PREVIOUS_INPUT={history:.8e}");
    eprintln!("INITIAL_RNG={} {}", initial.rng[0], initial.rng[1]);
    let names = [
        "frequency",
        "resonance",
        "cutoff_cv_amount",
        "resonance_cv_amount",
        "drive",
        "drive_cv_amount",
    ];
    let mut previous_controls = [f32::NAN; 6];
    let mut stdout = BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let row: Vec<f32> = line
            .split_whitespace()
            .map(|token| token.parse().unwrap())
            .collect();
        assert_eq!(row.len(), 77);
        assert!(row.iter().all(|value| value.is_finite()));
        let action = integer(row[0], 0, 2);
        let observe = integer(row[1], 0, 1) == 1;
        let mask = integer(row[2], 0, 3);
        let controls: [f32; 6] = row[3..9].try_into().unwrap();
        let changed = controls
            .iter()
            .zip(previous_controls)
            .any(|(value, previous)| value.to_bits() != previous.to_bits());
        if action == 2 || changed {
            node.params = names
                .iter()
                .zip(controls)
                .map(|(name, value)| (name.to_string(), value))
                .collect::<BTreeMap<_, _>>();
            let mut next = Rack264VcfProbe::new(&node, rate).unwrap();
            next.adopt(&module);
            module = next;
            previous_controls = controls;
        }
        if action == 1 {
            module.initialize();
        }
        let mut inputs = [PortFrame::default(); 4];
        for (port_index, port) in inputs.iter_mut().enumerate() {
            let start = 9 + 17 * port_index;
            port.channels = integer(row[start], 0, 16) as u8;
            port.values.copy_from_slice(&row[start + 1..start + 17]);
            assert!(port.values[usize::from(port.channels)..]
                .iter()
                .all(|value| value.to_bits() == 0));
        }
        let outputs = module.step(&inputs, mask);
        let width = inputs[3].channels.max(1);
        write!(stdout, "{width}").unwrap();
        macro_rules! emit {
            ($value:expr) => {{
                let value: f32 = $value;
                assert!(value.is_finite());
                write!(stdout, " {:.8e}", value).unwrap();
            }};
        }
        for (output_index, output) in outputs.iter().enumerate() {
            assert_eq!(output.channels, width);
            assert!(output.values[usize::from(width)..]
                .iter()
                .all(|value| value.to_bits() == 0));
            if mask & (1 << output_index) == 0 {
                assert!(output.values.iter().all(|value| value.to_bits() == 0));
            }
            for value in output.values {
                emit!(value);
            }
        }
        let snapshot = module.snapshot();
        // Integers never pass through f32 or decimal floating-point formatting.
        write!(stdout, " {} {}", snapshot.rng[0], snapshot.rng[1]).unwrap();
        if observe {
            for lane in snapshot.states {
                for value in lane {
                    emit!(value);
                }
            }
        }
        writeln!(stdout).unwrap();
    }
    stdout.flush().unwrap();
}
