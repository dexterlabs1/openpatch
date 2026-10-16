//! Text adapter to the production Random probe. No fixture-specific DSP.
use openpatch_core::{Node, PortFrame, RandomProbe};
use serde_json::json;
use std::io::{self, BufRead, Write};

fn run() -> Result<(), String> {
    let rate = std::env::args()
        .nth(1)
        .ok_or("missing sample rate")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let mut previous: Option<RandomProbe> = None;
    let mut previous_params: Option<[f32; 10]> = None;
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    for row in io::stdin().lock().lines() {
        let row = row.map_err(|e| e.to_string())?;
        if row.trim().is_empty() {
            continue;
        }
        let values: Vec<f32> = row
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if values.len() != 115 {
            return Err(format!("expected 115 fields, got {}", values.len()));
        }
        let event = values[0];
        if ![0.0, 1.0, 2.0, 3.0].contains(&event) {
            return Err("bad event".into());
        }
        let source: [f32; 10] = values[1..11].try_into().unwrap();
        if previous.is_none() || previous_params != Some(source) || event != 0.0 {
            let node: Node=serde_json::from_value(json!({"id":"random","kind":"random","params":{
                "rate":source[0],"shape":source[1],"unipolar":source[2],"probability":source[4],"spread":source[5],
                "rate_cv":source[6],"shape_cv":source[7],"probability_cv":source[8],"spread_cv":source[9],"seed":1
            }})).map_err(|e|e.to_string())?;
            let mut next = RandomProbe::new(&node, rate)?;
            if event != 2.0 {
                if let Some(old) = previous.as_ref() {
                    next.adopt(old);
                }
            }
            previous = Some(next);
            previous_params = Some(source);
        }
        let mut inputs = [PortFrame::default(); 6];
        for (index, input) in inputs.iter_mut().enumerate() {
            let start = 11 + index * 17;
            if values[start].fract() != 0.0 || !(0.0..=16.0).contains(&values[start]) {
                return Err("invalid channel count".into());
            }
            input.channels = values[start] as u8;
            input.values.copy_from_slice(&values[start + 1..start + 17]);
        }
        let mut draws = 0_usize;
        let probe = previous.as_mut().unwrap();
        let outputs = probe.step_with_uniform(&inputs, &mut || {
            assert!(draws < 2, "more than two draws per sample");
            let result = values[113 + draws];
            draws += 1;
            assert!(
                result.is_finite() && (0.0..1.0).contains(&result),
                "invalid uniform draw"
            );
            result
        });
        let state = probe.snapshot();
        let mut result = Vec::with_capacity(129);
        result.extend(outputs.iter().map(|out| f64::from(out.channels)));
        for out in outputs {
            result.extend(out.values.map(f64::from));
        }
        result.extend(state.last_voltage.map(f64::from));
        result.extend(state.next_voltage.map(f64::from));
        result.extend([
            f64::from(state.phase),
            f64::from(state.clock_frequency),
            f64::from(state.clock_phase),
            f64::from(state.timer),
            f64::from(state.pulse_remaining),
            f64::from(state.schmitt),
        ]);
        result.extend(state.lights.map(f64::from));
        result.push(draws as f64);
        for (index, value) in result.iter().enumerate() {
            if index > 0 {
                write!(writer, " ").map_err(|e| e.to_string())?;
            }
            write!(writer, "{value:.9e}").map_err(|e| e.to_string())?;
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
