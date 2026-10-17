//! Text-only reference access to the actual production StockVcoProbe.
//! No oscillator, reconstruction, kernel generation, or state transition is
//! reimplemented here. Bypass is the explicitly declared host-policy shim.
use openpatch_core::{Node, PortFrame, StockVcoProbe, StockVcoSnapshot};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};

fn emit(writer: &mut impl Write, values: impl IntoIterator<Item = f32>) -> Result<(), String> {
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            write!(writer, " ").map_err(|error| error.to_string())?;
        }
        write!(writer, "{:.9e}", f64::from(value)).map_err(|error| error.to_string())?;
    }
    writeln!(writer).map_err(|error| error.to_string())
}
fn initial(state: &StockVcoSnapshot) -> bool {
    state.phases.iter().all(|v| v.to_bits() == 0)
        && state.sync_directions.iter().all(|v| *v == 1.0)
        && state.last_sync.iter().all(|v| v.to_bits() == 0)
        && state.last_square.iter().all(|v| *v == 1.0)
        && state
            .filter_state
            .iter()
            .flatten()
            .all(|v| v.to_bits() == 0)
        && state
            .correction_tails
            .iter()
            .flatten()
            .flatten()
            .all(|v| v.to_bits() == 0)
        && state.buffer_indices.iter().flatten().all(|v| *v == 0)
        && state.light_divider == 0
        && state.lights.iter().all(|v| v.to_bits() == 0)
}
fn node(params: [f32; 6]) -> Node {
    Node {
        id: "vco".into(),
        kind: "stock_vco".into(),
        params: BTreeMap::from([
            ("frequency".into(), params[0]),
            ("fm_amount".into(), params[1]),
            ("pulse_width".into(), params[2]),
            ("pwm_amount".into(), params[3]),
            ("linear_fm".into(), params[4]),
            ("soft_sync".into(), params[5]),
        ]),
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
    }
}
fn integer(value: f32, maximum: u8, label: &str) -> Result<u8, String> {
    if !value.is_finite() || value.fract() != 0.0 || !(0.0..=f32::from(maximum)).contains(&value) {
        return Err(format!("invalid {label}"));
    }
    Ok(value as u8)
}
fn run() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (rate, mode) = match arguments.as_slice() {
        [flag] if flag == "--kernel" => (48000, Some("--kernel")),
        [rate] => (
            rate.parse::<u32>().map_err(|error| error.to_string())?,
            None,
        ),
        [rate, mode] if matches!(mode.as_str(), "--kernel" | "--safety") => (
            rate.parse::<u32>().map_err(|error| error.to_string())?,
            Some(mode.as_str()),
        ),
        _ => {
            return Err(
                "usage: stock_vco_reference_probe RATE [--safety|--kernel], or --kernel".into(),
            )
        }
    };
    let safety = mode == Some("--safety");
    let mut writer = io::BufWriter::new(io::stdout().lock());
    if mode == Some("--kernel") {
        let probe = StockVcoProbe::new(&node([0.0, 0.0, 0.5, 0.0, 0.0, 0.0]), rate)?;
        let kernel = probe.kernel();
        // The API stores oversample-major. The frozen wire is chronological.
        let mut values = Vec::with_capacity(1024);
        for table in [&kernel.impulse, &kernel.ramp] {
            for sample in 0..32 {
                for row in table {
                    values.push(row[sample]);
                }
            }
        }
        emit(&mut writer, values)?;
        return writer.flush().map_err(|error| error.to_string());
    }
    let mut previous: Option<StockVcoProbe> = None;
    let mut previous_params: Option<[f32; 6]> = None;
    for line in io::stdin().lock().lines() {
        let fields = line
            .map_err(|error| error.to_string())?
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        if fields.len() != 78 || (!safety && fields.iter().any(|v| !v.is_finite())) {
            return Err("expected 78 fields; nonfinite input diagnostics require --safety".into());
        }
        let event = integer(fields[0], 3, "event")?;
        let params: [f32; 6] = fields[1..7].try_into().unwrap();
        if previous.is_none() || matches!(event, 1 | 3) || previous_params != Some(params) {
            let mut receiving = StockVcoProbe::new(&node(params), rate)?;
            if !initial(&receiving.snapshot()) {
                return Err("fresh production snapshot violates frozen initial state".into());
            }
            if event != 1 {
                if let Some(old) = previous.as_ref() {
                    receiving.adopt(old);
                }
            }
            previous = Some(receiving);
            previous_params = Some(params);
        }
        let probe = previous.as_mut().unwrap();
        if event == 2 {
            probe.initialize();
        }
        let mut inputs = [PortFrame::default(); 4];
        for (index, input) in inputs.iter_mut().enumerate() {
            let start = 7 + 17 * index;
            input.channels = integer(fields[start], 16, "input width")?;
            input.values.copy_from_slice(&fields[start + 1..start + 17]);
            if input.values[usize::from(input.channels)..]
                .iter()
                .any(|v| v.to_bits() != 0)
            {
                return Err("inactive input slots must be positive zero".into());
            }
        }
        let mask = u16::from(integer(fields[75], 15, "consumer mask")?);
        let bypass = integer(fields[76], 1, "bypass")? != 0;
        let checkpoint = integer(fields[77], 1, "checkpoint")? != 0;
        // This shim matches generic OpenPatch Engine bypass and is separately
        // declared in acceptance. It is not a Rack Engine bypass measurement.
        let outputs = if bypass {
            [PortFrame::mono(0.0); 4]
        } else {
            probe.step(&inputs, mask)
        };
        let state = probe.snapshot();
        let mut values = Vec::with_capacity(if checkpoint { 2266 } else { 218 });
        values.extend(outputs.iter().map(|output| f32::from(output.channels)));
        for output in outputs {
            values.extend(output.values);
        }
        values.extend(state.phases);
        values.extend(state.sync_directions);
        values.extend(state.last_sync);
        values.extend(state.last_square);
        for filters in state.filter_state {
            values.extend(filters);
        }
        for indices in state.buffer_indices {
            values.extend(indices.map(f32::from));
        }
        values.push(f32::from(state.light_divider));
        values.extend(state.lights);
        assert_eq!(values.len(), 218);
        if checkpoint {
            for output in state.correction_tails {
                for lane in output {
                    values.extend(lane);
                }
            }
        }
        assert_eq!(values.len(), if checkpoint { 2266 } else { 218 });
        emit(&mut writer, values)?;
    }
    writer.flush().map_err(|error| error.to_string())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
