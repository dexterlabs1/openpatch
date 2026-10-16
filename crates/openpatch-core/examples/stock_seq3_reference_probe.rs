//! Frozen text protocol around the actual production StockSeq3Probe. No DSP mirror.
use openpatch_core::{Node, PortFrame, SavedModuleState, StockSeq3Probe, StockSeq3Snapshot};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
fn integer(value: f32, min: i32, max: i32, label: &str) -> Result<i32, String> {
    if !value.is_finite() || value.fract() != 0.0 || value < min as f32 || value > max as f32 {
        return Err(format!("invalid {label}"));
    }
    Ok(value as i32)
}
fn node(params: &[f32; 28]) -> Node {
    let mut values = BTreeMap::from([
        ("tempo".into(), params[0]),
        ("tempo_amount".into(), params[1]),
        ("steps".into(), params[2]),
        ("steps_amount".into(), params[3]),
    ]);
    for row in 0..3 {
        for step in 0..8 {
            values.insert(
                format!("row{}_{}", row + 1, step + 1),
                params[4 + row * 8 + step],
            );
        }
    }
    Node {
        id: "seq".into(),
        kind: "stock_seq3".into(),
        params: values,
        content: None,
        score: None,
        bypassed: false,
    }
}
fn fresh(state: &StockSeq3Snapshot) -> bool {
    state.phase.to_bits() == 0
        && state.index == 0
        && state.running
        && state.gates == [true; 8]
        && !state.passthrough
        && state.button_triggers == [2; 11]
        && state.input_triggers == [2; 3]
        && state
            .pulse_remaining
            .iter()
            .chain(state.lights.iter())
            .all(|v| v.to_bits() == 0)
        && state.pressed_mask == 0
}
fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let rate = arguments
        .first()
        .ok_or("missing sample rate")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let safety = match arguments.get(1).map(String::as_str) {
        None => false,
        Some("--safety") => true,
        _ => return Err("unknown argument".into()),
    };
    if arguments.len() > 2 {
        return Err("unexpected arguments".into());
    }
    let mut writer = io::BufWriter::new(io::stdout().lock());
    let mut previous: Option<StockSeq3Probe> = None;
    let mut previous_params: Option<[f32; 28]> = None;
    for line in io::stdin().lock().lines() {
        let fields = line
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if fields.len() != 120 || (!safety && fields.iter().any(|v| !v.is_finite())) {
            return Err("expected120fields; nonfinite CV requires --safety".into());
        }
        let event = integer(fields[0], 0, 5, "event")?;
        let params: [f32; 28] = fields[1..29].try_into().unwrap();
        if previous.is_none() || event == 1 || event == 3 || previous_params != Some(params) {
            let mut next = StockSeq3Probe::new(&node(&params), rate)?;
            if !fresh(&next.snapshot()) {
                return Err("fresh production snapshot differs from frozen contract".into());
            }
            if event != 1 {
                if let Some(old) = previous.as_ref() {
                    next.adopt(old);
                }
            }
            previous = Some(next);
            previous_params = Some(params);
        }
        let probe = previous.as_mut().unwrap();
        if event == 2 {
            probe.initialize();
        }
        let held = integer(fields[29], 0, 2047, "held mask")? as u16;
        let mut inputs = [PortFrame::default(); 5];
        for (i, input) in inputs.iter_mut().enumerate() {
            let at = 30 + 17 * i;
            input.channels = integer(fields[at], 0, 16, "input width")? as u8;
            input.values.copy_from_slice(&fields[at + 1..at + 17]);
            if input.values[usize::from(input.channels)..]
                .iter()
                .any(|v| v.to_bits() != 0)
            {
                return Err("inactive input slots must be positive zero".into());
            }
        }
        let running = integer(fields[115], -1, 1, "durable running")?;
        let gates = integer(fields[116], -1, 255, "durable gates")?;
        let passthrough = integer(fields[117], -1, 1, "durable passthrough")?;
        if event == 4 {
            let state = probe.snapshot();
            let previous_gates = state
                .gates
                .iter()
                .enumerate()
                .fold(0_u8, |mask, (i, value)| mask | (u8::from(*value) << i));
            // dataFromJson field-presence mapping only. The legacy omitted
            // passthrough fallback is deliberately distinct from a fresh module.
            let saved = SavedModuleState::StockSeq3 {
                running: if running < 0 {
                    state.running
                } else {
                    running != 0
                },
                gates: if gates < 0 {
                    previous_gates
                } else {
                    gates as u8
                },
                passthrough: if passthrough < 0 {
                    true
                } else {
                    passthrough != 0
                },
            };
            if !probe.set_saved_state(saved) {
                return Err("production rejected durable state".into());
            }
        }
        let rotation = integer(fields[118], -8, 8, "rotation")?;
        if event == 5 {
            probe.rotate(rotation);
            previous_params = None;
        }
        let bypass = integer(fields[119], 0, 1, "bypass")? != 0;
        let outputs = if bypass {
            [PortFrame::mono(0.0); 16]
        } else {
            probe.step(&inputs, held)
        };
        let state = probe.snapshot();
        let mut values = Vec::with_capacity(90);
        values.extend(outputs.iter().map(|output| f32::from(output.channels)));
        values.extend(outputs.iter().map(|output| output.values[0]));
        values.extend([
            state.phase,
            f32::from(state.index),
            f32::from(state.length),
            f32::from(u8::from(state.running)),
        ]);
        values.extend(state.gates.map(|value| f32::from(u8::from(value))));
        values.push(f32::from(u8::from(state.passthrough)));
        values.extend(state.button_triggers.map(f32::from));
        values.extend(state.input_triggers.map(f32::from));
        values.extend(state.pulse_remaining);
        values.extend(state.lights);
        values.push(f32::from(state.pressed_mask));
        assert_eq!(values.len(), 90);
        for (i, value) in values.into_iter().enumerate() {
            if i > 0 {
                write!(writer, " ").map_err(|e| e.to_string())?;
            }
            write!(writer, "{:.9e}", f64::from(value)).map_err(|e| e.to_string())?;
        }
        writeln!(writer).map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
