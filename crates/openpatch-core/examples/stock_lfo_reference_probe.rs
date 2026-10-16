//! Text adapter to actual production StockLfoProbe; no fixture-specific DSP.
use openpatch_core::{Node, PortFrame, StockLfoProbe};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

fn run() -> Result<(), String> {
    let rate: u32 = std::env::args()
        .nth(1)
        .ok_or("missing rate")?
        .parse::<u32>()
        .map_err(|e| e.to_string())?;
    let safety = match std::env::args().nth(2).as_deref() {
        None => false,
        Some("--safety") => true,
        _ => return Err("unknown argument".into()),
    };
    let mut previous: Option<StockLfoProbe> = None;
    let mut previous_params: Option<[f32; 6]> = None;
    let mut writer = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let fields: Vec<f32> = line
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        if fields.len() != 76 || (!safety && fields.iter().any(|v| !v.is_finite())) {
            return Err("expected76 finite fields".into());
        }
        let event = fields[0];
        if ![0.0, 1.0, 2.0].contains(&event) {
            return Err("invalid event".into());
        }
        let params: [f32; 6] = fields[1..7].try_into().unwrap();
        if previous.is_none() || event == 1.0 || previous_params != Some(params) {
            let node = Node {
                id: "lfo".into(),
                kind: "stock_lfo".into(),
                params: BTreeMap::from([
                    ("rate".into(), params[0]),
                    ("fm_amount".into(), params[1]),
                    ("pulse_width".into(), params[2]),
                    ("pwm_amount".into(), params[3]),
                    ("offset".into(), params[4]),
                    ("invert".into(), params[5]),
                ]),
                bypassed: false,
                content: None,
                score: None,
            };
            let mut next = StockLfoProbe::new(&node, rate)?;
            let initial = next.snapshot();
            if initial.phases.iter().any(|v| v.to_bits() != 0)
                || initial.reset_high.iter().any(|v| !*v)
                || initial.clock_trigger != 2
                || initial.clock_frequency != 1.0
                || initial.clock_timer != 0.0
                || initial.light_divider != 0
                || initial.lights.iter().any(|v| v.to_bits() != 0)
            {
                return Err("fresh snapshot differs from declared initial state".into());
            }
            if event != 1.0 {
                if let Some(old) = previous.as_ref() {
                    next.adopt(old);
                }
            }
            previous = Some(next);
            previous_params = Some(params);
        }
        let probe = previous.as_mut().unwrap();
        if event == 2.0 {
            probe.initialize();
        }
        let mut inputs = [PortFrame::default(); 4];
        for (i, input) in inputs.iter_mut().enumerate() {
            let at = 7 + i * 17;
            let width = fields[at];
            if width.fract() != 0.0 || !(0.0..=16.0).contains(&width) {
                return Err("bad input width".into());
            }
            input.channels = width as u8;
            input.values.copy_from_slice(&fields[at + 1..at + 17]);
            if input.values[width as usize..]
                .iter()
                .any(|v| v.to_bits() != 0)
            {
                return Err("inactive input storage must be positivezero".into());
            }
        }
        let mask = fields[75];
        if mask.fract() != 0.0 || !(0.0..=15.0).contains(&mask) {
            return Err("bad consumer mask".into());
        }
        let outputs = probe.step(&inputs, mask as u16);
        let state = probe.snapshot();
        let mut values = Vec::with_capacity(109);
        values.extend(outputs.iter().map(|o| f64::from(o.channels)));
        for output in outputs {
            values.extend(output.values.map(f64::from));
        }
        values.extend(state.phases.map(f64::from));
        values.extend(state.reset_high.map(|high| f64::from(u8::from(high))));
        values.extend([
            f64::from(state.clock_trigger),
            f64::from(state.clock_frequency),
            f64::from(state.clock_timer),
            f64::from(state.light_divider),
        ]);
        values.extend(state.lights.map(f64::from));
        assert_eq!(values.len(), 109);
        for (index, value) in values.into_iter().enumerate() {
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
