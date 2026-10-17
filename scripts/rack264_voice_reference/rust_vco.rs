// SPDX-License-Identifier: MIT
// Original observation adapter for the compiled production openpatch_core crate.
// No copied DSP source, stand-in Node/PortFrame, or second oscillator model.
use openpatch_core::{Node, PortFrame, Rack264VcoProbe};
use std::collections::BTreeMap;
use std::io::{self, BufRead, BufWriter, Write};

fn integer(value: f32, low: u16, high: u16) -> u16 {
    assert!(value.is_finite() && value.fract() == 0.0);
    assert!(value >= f32::from(low) && value <= f32::from(high));
    value as u16
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 2);
    let rate: u32 = args[1].parse().expect("sample_rate must be an integer");
    let mut node: Node = serde_json::from_value(serde_json::json!({
        "id": "reference", "kind": "rack264_vco", "params": {}
    }))
    .unwrap();
    let mut module = Rack264VcoProbe::new(&node, rate).unwrap();
    let names = [
        "frequency",
        "pulse_width",
        "fm_amount",
        "pwm_amount",
        "linear_fm",
        "soft_sync",
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
        let mask = integer(row[2], 0, 15);
        let controls: [f32; 6] = row[3..9].try_into().unwrap();
        // The probe intentionally has no ad-hoc parameter setter. Refreshing a
        // changed control graph exercises the same production build/adopt path.
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
            let mut next = Rack264VcoProbe::new(&node, rate).unwrap();
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
                .all(|v| v.to_bits() == 0));
        }
        let outputs = module.step(&inputs, mask);
        let width = inputs[0].channels.max(1);
        write!(stdout, "{width}").unwrap();
        // Nine significant decimal digits round-trip every finite f32.
        macro_rules! emit {
            ($value:expr) => {
                write!(stdout, " {:.8e}", $value).unwrap()
            };
        }
        for (wave, output) in outputs.iter().enumerate() {
            assert_eq!(
                output.channels,
                if mask & (1 << wave) != 0 { width } else { 0 }
            );
            assert!(output.values[usize::from(output.channels)..]
                .iter()
                .all(|v| v.to_bits() == 0));
            for value in output.values {
                emit!(value);
            }
        }
        if observe {
            let snapshot = module.snapshot();
            for field in [
                snapshot.phases,
                snapshot.frequencies,
                snapshot.pulse_widths,
                snapshot.sync_directions,
                snapshot.last_sync,
                snapshot.square_states,
                snapshot.filter_input,
                snapshot.filter_output,
            ] {
                for value in field {
                    emit!(value);
                }
            }
            for wave in snapshot.correction_tails {
                for lane in wave {
                    for value in lane {
                        emit!(value);
                    }
                }
            }
            for index in snapshot.buffer_indices {
                write!(stdout, " {index}").unwrap();
            }
            for wave in snapshot.wave_values {
                for value in wave {
                    emit!(value);
                }
            }
            write!(stdout, " {}", snapshot.light_divider).unwrap();
            for value in snapshot.lights {
                emit!(value);
            }
        }
        writeln!(stdout).unwrap();
    }
    stdout.flush().unwrap();
}
