//! Text adapter over the actual fixed-storage production lifecycle transform.
use openpatch_core::OutputLifecycleTransform;
use std::io::{self, BufRead, Write};

fn integer(value: f32, max: u16) -> Result<u16, String> {
    if !value.is_finite() || value < 0.0 || value > f32::from(max) || value.fract() != 0.0 {
        return Err("invalid protocol integer".into());
    }
    Ok(value as u16)
}

fn run() -> Result<(), String> {
    let mut writer = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let fields = line
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .map(str::parse::<f32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if fields.len() < 70 || fields.iter().any(|v| !v.is_finite()) {
            return Err("expected at least70 finite fields".into());
        }
        let mut mask = integer(fields[0], 255)? as u8;
        let mut counts = [0u8; 4];
        let mut raw = [[0.0; 16]; 4];
        for p in 0..4 {
            counts[p] = integer(fields[1 + p], 16)? as u8;
            if (counts[p] > 0) != (mask & (3 << (p * 2)) != 0) {
                return Err("incoherent initial consumers/counts".into());
            }
            raw[p].copy_from_slice(&fields[5 + p * 16..21 + p * 16]);
        }
        let length = usize::from(integer(fields[69], 4096)?);
        if fields.len() != 70 + length {
            return Err("wrong operation count".into());
        }
        let mut transforms = [OutputLifecycleTransform::identity(); 4];
        let mut bypassed = false;
        for &field in &fields[70..] {
            let op = integer(field, 18)? as u8;
            if op < 16 {
                let cable = op & 7;
                let port = usize::from(cable / 2);
                let before = mask & (3 << (port * 2)) != 0;
                if op < 8 {
                    mask |= 1 << cable;
                } else {
                    mask &= !(1 << cable);
                }
                let after = mask & (3 << (port * 2)) != 0;
                if before != after {
                    transforms[port] = transforms[port].then(if after {
                        OutputLifecycleTransform::connect()
                    } else {
                        OutputLifecycleTransform::disconnect()
                    });
                }
            } else {
                let next = if op == 16 { !bypassed } else { op == 18 };
                if bypassed != next {
                    for t in &mut transforms {
                        *t = t.then(OutputLifecycleTransform::bypass_edge());
                    }
                    bypassed = next;
                }
            }
        }
        for p in 0..4 {
            transforms[p]
                .apply(&mut counts[p], &mut raw[p])
                .map_err(|e| e.to_string())?;
        }
        let mut values = Vec::with_capacity(70);
        values.extend(counts.map(f32::from));
        values.extend(raw.into_iter().flatten());
        values.push(f32::from(u8::from(bypassed)));
        values.push(f32::from(mask));
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                write!(writer, " ").map_err(|e| e.to_string())?;
            }
            write!(writer, "{value:?}").map_err(|e| e.to_string())?;
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
