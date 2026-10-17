//! Text adapter for the actual production StockQuantizerProbe; no DSP mirror.
use openpatch_core::{Node, PortFrame, StockQuantizerProbe};
use std::collections::BTreeMap;
use std::io::{self, BufRead, Write};
fn integer(value: f32, min: i32, max: i32, label: &str) -> Result<i32, String> {
    if !value.is_finite() || value.fract() != 0.0 || value < min as f32 || value > max as f32 {
        return Err(format!("invalid {label}"));
    }
    Ok(value as i32)
}
fn node(mask: u16) -> Node {
    Node {
        id: "q".into(),
        kind: "stock_quantizer".into(),
        params: BTreeMap::from([("scale_mask".into(), f32::from(mask))]),
        content: None,
        rackmcp_sequence: None,
        score: None,
        bypassed: false,
    }
}
fn fresh(rate: u32) -> Result<StockQuantizerProbe, String> {
    let probe = StockQuantizerProbe::new(&node(4095), rate)?;
    let state = probe.snapshot();
    if state.enabled_mask != 4095 || state.playing_notes.is_some() || state.offset.to_bits() != 0 {
        return Err("fresh production snapshot violates frozen initial-state contract".into());
    }
    Ok(probe)
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
    let mut previous: Option<StockQuantizerProbe> = None;
    for line in io::stdin().lock().lines() {
        let fields = line
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if fields.len() != 24 || (!safety && fields.iter().any(|value| !value.is_finite())) {
            return Err("expected24fields; nonfinite raw CV requires --safety".into());
        }
        let event = integer(fields[0], 0, 6, "event")?;
        let channels = integer(fields[2], 0, 16, "input width")? as u8;
        let mut input = PortFrame {
            channels,
            ..Default::default()
        };
        input.values.copy_from_slice(&fields[3..19]);
        if input.values[usize::from(channels)..]
            .iter()
            .any(|value| value.to_bits() != 0)
        {
            return Err("inactive input slots must be positive zero".into());
        }
        let length = integer(fields[19], -1, 14, "legacy array length")?;
        let mask = integer(fields[20], 0, 16383, "boolean mask")? as u16;
        let rotation = integer(fields[21], -12, 12, "rotation")?;
        let connected = integer(fields[22], 0, 1, "output connected")? != 0;
        let bypassed = integer(fields[23], 0, 1, "bypassed")? != 0;
        if previous.is_none() || event == 1 {
            previous = Some(fresh(rate)?);
        }
        if event == 3 || event == 6 {
            let old = previous.as_ref().unwrap();
            let receiving_mask = if event == 3 {
                old.snapshot().enabled_mask
            } else {
                mask & 0x0fff
            };
            let mut receiving = StockQuantizerProbe::new(&node(receiving_mask), rate)?;
            receiving.adopt(old);
            previous = Some(receiving);
        }
        let probe = previous.as_mut().unwrap();
        probe.set_offset(fields[1])?;
        match event {
            2 => probe.initialize(),
            4 => probe.load_legacy_notes(length, mask)?,
            5 => probe.rotate(rotation),
            _ => {}
        }
        // Counts and bypass behavior are actual production results. In particular,
        // disconnected outputs are not rewritten by the text adapter.
        let output = probe.step(&input, connected, bypassed);
        let state = probe.snapshot();
        let mut values = Vec::with_capacity(45);
        values.push(f32::from(output.channels));
        values.extend(output.values);
        values.push(f32::from(state.enabled_mask));
        values.extend(state.ranges.map(|value| value as f32));
        values.push(f32::from(u8::from(state.playing_notes.is_some())));
        values.push(f32::from(state.playing_notes.unwrap_or(0)));
        values.push(state.offset);
        assert_eq!(values.len(), 45);
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
